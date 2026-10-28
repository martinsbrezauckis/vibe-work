//! Files reader checks using generated repositories. No window or external service is needed.
use crate::explorer_view::ExplorerResult;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Folder(PathBuf);
impl Folder {
    fn new() -> Self {
        let p = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "vw-files-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        vw_changes::prepare_root(&p.join(".data")).unwrap();
        Self(p)
    }
    fn git(&self, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.0)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    fn init(&self) {
        self.git(&["init", "-q"]);
        self.git(&["config", "user.name", "Sample"]);
        self.git(&["config", "user.email", "sample@example.test"]);
    }
}
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn one_level_read_hides_ignored_and_never_enters_children() {
    let f = Folder::new();
    f.init();
    std::fs::write(f.0.join(".gitignore"), "node_modules/\n").unwrap();
    std::fs::create_dir_all(f.0.join("src/deep")).unwrap();
    std::fs::create_dir_all(f.0.join("node_modules")).unwrap();
    std::fs::write(f.0.join("src/deep/ābols file.rs"), "hello").unwrap();
    let ExplorerResult::Folder {
        entries, ignored, ..
    } = crate::explorer_read::folder(&f.0, "", false, 0).unwrap()
    else {
        panic!()
    };
    assert!(entries.iter().all(|e| !e.path.contains('/')));
    assert!(entries.iter().any(|e| e.path == "src"));
    assert!(ignored.iter().any(|p| p == "node_modules"));
    assert!(!entries.iter().any(|e| e.path == "node_modules"));
    let ExplorerResult::Folder { entries, .. } =
        crate::explorer_read::folder(&f.0, "", true, 0).unwrap()
    else {
        panic!()
    };
    assert!(entries.iter().any(|e| e.path == "node_modules"));
    assert!(crate::explorer_read::folder(&f.0, "../elsewhere", false, 0).is_err());
}
#[test]
fn name_index_keeps_spaces_unicode_and_git_ignore_rules() {
    let f = Folder::new();
    f.init();
    std::fs::write(f.0.join("ābols file.rs"), "hello").unwrap();
    std::fs::write(f.0.join(".gitignore"), "ignored\n").unwrap();
    std::fs::write(f.0.join("ignored"), "hidden").unwrap();
    let ExplorerResult::Index { paths, .. } = crate::explorer_read::index(&f.0) else {
        panic!()
    };
    assert!(paths.iter().any(|p| p == "ābols file.rs"));
    assert!(!paths.iter().any(|p| p == "ignored"));
}
#[test]
fn plain_folders_have_bounded_search_and_cancellation() {
    let f = Folder::new();
    std::fs::write(f.0.join("notes.txt"), "hello\n".repeat(800)).unwrap();
    let ExplorerResult::Search { hits, truncated } =
        crate::explorer_read::search(&f.0, "hello", None, None).unwrap()
    else {
        panic!()
    };
    assert_eq!(hits.len(), 500);
    assert!(truncated);
    let cancel = AtomicBool::new(true);
    assert!(matches!(
        crate::explorer_read::search(&f.0, "hello", None, Some(&cancel)).unwrap(),
        ExplorerResult::Done {}
    ));
}
#[test]
fn history_cursor_stays_on_its_original_head_when_a_commit_arrives() {
    let f = Folder::new();
    f.init();
    std::fs::write(f.0.join("notes.txt"), "hello").unwrap();
    f.git(&["add", "."]);
    f.git(&["commit", "-qm", "Start"]);
    for _ in 0..54 {
        f.git(&["commit", "--allow-empty", "-qm", "Next"]);
    }
    let ExplorerResult::History { versions, cursor } =
        crate::explorer_history::history(&f.0, None, "", None).unwrap()
    else {
        panic!()
    };
    assert_eq!(versions.len(), 50);
    f.git(&["commit", "--allow-empty", "-qm", "Arrived later"]);
    let ExplorerResult::History {
        versions: next,
        cursor: None,
    } = crate::explorer_history::history(&f.0, cursor.as_deref(), "", None).unwrap()
    else {
        panic!()
    };
    assert_eq!(next.len(), 5);
    assert!(next.iter().all(|v| v.subject != "Arrived later"));
    assert!(
        next.iter()
            .all(|v| !versions.iter().any(|old| old.id == v.id))
    );
    let ExplorerResult::Version { files, .. } =
        crate::explorer_history::version(&f.0, &next.last().unwrap().id).unwrap()
    else {
        panic!()
    };
    assert_eq!(files, vec!["notes.txt"]);
}
#[test]
fn commit_search_matches_message_or_author() {
    let f = Folder::new();
    f.init();
    f.git(&["commit", "--allow-empty", "-qm", "Explain multiply"]);
    for query in ["multiply", "Sample"] {
        let ExplorerResult::History { versions, .. } =
            crate::explorer_history::history(&f.0, None, query, None).unwrap()
        else {
            panic!()
        };
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].subject, "Explain multiply");
    }
}

#[test]
fn large_directory_is_paged_and_paths_do_not_traverse() {
    let f = Folder::new();
    for n in 0..5010 {
        std::fs::write(f.0.join(format!("f{n}")), "").unwrap();
    }
    let ExplorerResult::Folder { entries, next, .. } =
        crate::explorer_read::folder(&f.0, "", false, 0).unwrap()
    else {
        panic!()
    };
    assert_eq!(entries.len(), 5000);
    assert_eq!(next, Some(5000));
    for p in ["../file", "/etc/passwd", "src/../../file", ".git/config"] {
        assert!(!crate::explorer_read::relative(p));
    }
    assert!(crate::explorer_read::relative("src/ābols file.rs"));
    let _ = Path::new("");
}

#[test]
fn git_text_search_returns_lines_and_stops_at_the_cap() {
    let f = Folder::new();
    f.init();
    std::fs::write(f.0.join("spaced ābols.rs"), "multiply\n".repeat(800)).unwrap();
    let ExplorerResult::Search { hits, truncated } =
        crate::explorer_read::search(&f.0, "multiply", None, None).unwrap()
    else {
        panic!()
    };
    assert_eq!(hits.len(), 500);
    assert!(truncated);
    assert_eq!(hits[0].path, "spaced ābols.rs");
    assert_eq!(hits[0].line, 1);
    let cancelled = AtomicBool::new(true);
    let started = std::time::Instant::now();
    assert!(matches!(
        crate::explorer_read::search(&f.0, "multiply", None, Some(&cancelled)).unwrap(),
        ExplorerResult::Done {}
    ));
    assert!(started.elapsed() < std::time::Duration::from_millis(50));
}

#[test]
fn text_search_in_a_folder_counts_the_cap_there_and_refuses_paths_outside() {
    for git in [true, false] {
        let f = Folder::new();
        if git {
            f.init();
        }
        // 600 hits in a folder that sorts first fill the 500 cap before src/ is reached.
        std::fs::create_dir_all(f.0.join("docs")).unwrap();
        std::fs::create_dir_all(f.0.join("src")).unwrap();
        std::fs::write(f.0.join("docs/a.md"), "multiply\n".repeat(600)).unwrap();
        std::fs::write(f.0.join("src/calc.rs"), "fn multiply() {}\n").unwrap();
        let ExplorerResult::Search { hits, truncated } =
            crate::explorer_read::search(&f.0, "multiply", None, None).unwrap()
        else {
            panic!()
        };
        assert!(truncated && hits.len() == 500, "git {git}");
        if git {
            // The whole-folder search cannot show the folder's own hit: the reason for the folder argument.
            assert!(hits.iter().all(|h| h.path == "docs/a.md"));
        }
        let ExplorerResult::Search { hits, truncated } =
            crate::explorer_read::search(&f.0, "multiply", Some("src"), None).unwrap()
        else {
            panic!()
        };
        assert!(!truncated, "git {git}");
        assert_eq!(
            hits.iter()
                .map(|h| (h.path.as_str(), h.line))
                .collect::<Vec<_>>(),
            vec![("src/calc.rs", 1)],
            "git {git}"
        );
        let outside = Folder::new();
        std::fs::write(outside.0.join("x.rs"), "multiply\n").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside.0, f.0.join("link")).unwrap();
        for path in ["../elsewhere", "/etc", "src/../../x", ".git", "link"] {
            assert!(
                crate::explorer_read::search(&f.0, "multiply", Some(path), None).is_err(),
                "git {git}: {path} was searched"
            );
        }
    }
}

#[test]
fn folder_review_rejects_changes_after_confirmation_and_deletes_only_named_new_files() {
    let f = Folder::new();
    f.init();
    std::fs::write(f.0.join(".gitignore"), ".data/\n").unwrap();
    std::fs::write(f.0.join("notes.txt"), "before\n").unwrap();
    f.git(&["add", "."]);
    f.git(&["commit", "-qm", "Start"]);
    std::fs::write(f.0.join("notes.txt"), "after\n").unwrap();
    std::fs::write(f.0.join("new.txt"), "new\n").unwrap();
    let (_, diff) = crate::explorer_history::review(&f.0, &f.0.join(".data")).unwrap();
    assert_eq!(diff.files.len(), 2);
    let head =
        String::from_utf8(crate::explorer_read::git(&f.0, &["rev-parse", "HEAD"], 256).unwrap())
            .unwrap();
    std::fs::write(f.0.join("notes.txt"), "arrived later\n").unwrap();
    assert!(
        crate::explorer_history::revert(
            &f.0,
            &f.0.join(".data"),
            head.trim(),
            &diff.fingerprint,
            &["new.txt".into()]
        )
        .is_err()
    );
    assert!(f.0.join("new.txt").exists());
    let (_, diff) = crate::explorer_history::review(&f.0, &f.0.join(".data")).unwrap();
    assert!(
        crate::explorer_history::revert(
            &f.0,
            &f.0.join(".data"),
            head.trim(),
            &diff.fingerprint,
            &[]
        )
        .is_err()
    );
    crate::explorer_history::revert(
        &f.0,
        &f.0.join(".data"),
        head.trim(),
        &diff.fingerprint,
        &["new.txt".into()],
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(f.0.join("notes.txt")).unwrap(),
        "before\n"
    );
    assert!(!f.0.join("new.txt").exists());
}

#[test]
#[ignore = "generates 80,000 files and 50,000 commits for timing assertions"]
fn generated_large_repository_meets_read_budgets() {
    use std::fmt::Write as _;
    use std::io::Write;
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    let f = Folder::new();
    f.init();
    f.git(&["branch", "-M", "main"]);
    for d in 0..16 {
        let dir = f.0.join(format!("folder{d}"));
        std::fs::create_dir(&dir).unwrap();
        for n in 0..5000 {
            std::fs::write(dir.join(format!("file{n}.rs")), "multiply\n").unwrap();
        }
    }
    std::fs::write(f.0.join(".gitignore"), ".data/\n").unwrap();
    f.git(&["add", "."]);
    f.git(&["commit", "-qm", "Start"]);
    let head =
        String::from_utf8(crate::explorer_read::git(&f.0, &["rev-parse", "HEAD"], 256).unwrap())
            .unwrap();
    let mut input = String::new();
    for n in 1..=50_000 {
        write!(input, "commit refs/heads/main\nmark :{n}\ncommitter Sample <sample@example.test> {} +0000\ndata 7\nVersion\nfrom {}\n\n",1_700_000_000+n,if n==1 {head.trim().to_owned()}else{format!(":{}",n-1)}).unwrap();
    }
    input.push_str("done\n");
    let mut child = Command::new("git")
        .arg("-C")
        .arg(&f.0)
        .args(["fast-import", "--quiet"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
    let start = Instant::now();
    crate::explorer_read::folder(&f.0, "", false, 0).unwrap();
    let root = start.elapsed();
    assert!(root < Duration::from_millis(200), "root {root:?}");
    let start = Instant::now();
    crate::explorer_read::folder(&f.0, "folder0", false, 0).unwrap();
    let folder = start.elapsed();
    assert!(folder < Duration::from_millis(300), "folder {folder:?}");
    let start = Instant::now();
    let ExplorerResult::Index { paths, .. } = crate::explorer_read::index(&f.0) else {
        panic!()
    };
    let index = start.elapsed();
    assert!(paths.len() >= 80_000);
    let start = Instant::now();
    let ExplorerResult::History { versions, cursor } =
        crate::explorer_history::history(&f.0, None, "", None).unwrap()
    else {
        panic!()
    };
    let first = start.elapsed();
    assert_eq!(versions.len(), 50);
    assert!(first < Duration::from_millis(400), "history {first:?}");
    let start = Instant::now();
    crate::explorer_history::history(&f.0, cursor.as_deref(), "", None).unwrap();
    let next = start.elapsed();
    assert!(next < Duration::from_millis(300), "next {next:?}");
    let start = Instant::now();
    crate::explorer_read::search(&f.0, "multiply", None, None).unwrap();
    let search = start.elapsed();
    assert!(search < Duration::from_millis(500), "search {search:?}");
    println!(
        "large fixture: 80,000 files, 50,001 commits; root={root:?}, folder={folder:?}, index={index:?}, history={first:?}, next={next:?}, text={search:?}"
    );
}
#[test]
fn history_of_one_file_lists_only_the_commits_that_changed_it() {
    let f = Folder::new();
    f.init();
    std::fs::create_dir_all(f.0.join("src")).unwrap();
    std::fs::write(f.0.join("src/calc.rs"), "one").unwrap();
    std::fs::write(f.0.join("notes.txt"), "a").unwrap();
    f.git(&["add", "."]);
    f.git(&["commit", "-qm", "Start"]);
    std::fs::write(f.0.join("notes.txt"), "b").unwrap();
    f.git(&["commit", "-qam", "Notes only"]);
    std::fs::write(f.0.join("src/calc.rs"), "two").unwrap();
    f.git(&["commit", "-qam", "Calc again"]);
    let ExplorerResult::History { versions, .. } =
        crate::explorer_history::history(&f.0, None, "", Some("src/calc.rs")).unwrap()
    else {
        panic!()
    };
    let subjects: Vec<_> = versions.iter().map(|v| v.subject.as_str()).collect();
    assert_eq!(subjects, ["Calc again", "Start"]);
    // A path that climbs out of the folder, or names Git's own data, is refused.
    assert!(crate::explorer_history::history(&f.0, None, "", Some("../x")).is_err());
    assert!(crate::explorer_history::history(&f.0, None, "", Some(".git/config")).is_err());
}
#[test]
fn a_file_reads_as_it_was_in_a_commit_also_from_a_subfolder() {
    let f = Folder::new();
    f.init();
    std::fs::create_dir_all(f.0.join("src")).unwrap();
    std::fs::write(f.0.join("src/calc.rs"), "fn old() {}\r\n").unwrap();
    f.git(&["add", "."]);
    f.git(&["commit", "-qm", "Start"]);
    std::fs::write(f.0.join("src/calc.rs"), "fn new() {}\n").unwrap();
    f.git(&["commit", "-qam", "New"]);
    let ExplorerResult::History { versions, .. } =
        crate::explorer_history::history(&f.0, None, "", None).unwrap()
    else {
        panic!()
    };
    let start = &versions.last().unwrap().id;
    let ExplorerResult::Text { text, note } =
        crate::explorer_history::read_at(&f.0, start, "src/calc.rs").unwrap()
    else {
        panic!()
    };
    assert_eq!((text.as_deref(), note), (Some("fn old() {}\n"), None));
    // Files may show a folder inside the repository: the path is relative to that folder.
    let ExplorerResult::Text { text, .. } =
        crate::explorer_history::read_at(&f.0.join("src"), start, "calc.rs").unwrap()
    else {
        panic!()
    };
    assert_eq!(text.as_deref(), Some("fn old() {}\n"));
    let ExplorerResult::Text { text: None, note } =
        crate::explorer_history::read_at(&f.0, start, "missing.rs").unwrap()
    else {
        panic!()
    };
    assert_eq!(note.as_deref(), Some("This file is not in that commit."));
    assert!(crate::explorer_history::read_at(&f.0, "HEAD", "src/calc.rs").is_err());
    assert!(crate::explorer_history::read_at(&f.0, start, "../calc.rs").is_err());
}
