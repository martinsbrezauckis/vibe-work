//! Agent search regression tests: real files, ignores, confinement and deterministic budgets.
use super::*;
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn an_opened_check_crossing_the_deadline_cannot_return_a_last_name_hit() {
    let t = Tree::new();
    t.put("last.txt", "needle");
    for cleared in [true, false] {
        let answer = run(
            &[Folder {
                path: t.0.clone(),
                session_id: None,
            }],
            &serde_json::from_value(json!({"glob":"*.txt"})).unwrap(),
            &|_| true,
            &|_, _| {
                std::thread::sleep(Duration::from_millis(100));
                cleared
            },
            Duration::from_millis(50),
        )
        .unwrap();
        assert_eq!(answer, "Search limit reached.");
    }
}

struct Tree(PathBuf);
impl Tree {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "vw-search-agent-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(vw_platform::canonical_plain(&p).unwrap())
    }
    fn put(&self, path: &str, text: &str) {
        let p = self.0.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    fn search(&self, args: serde_json::Value, allowed: &dyn Fn(&Path) -> bool) -> String {
        run(
            &[Folder {
                path: self.0.clone(),
                session_id: None,
            }],
            &serde_json::from_value(args).unwrap(),
            allowed,
            &|_, _| true,
            Duration::from_secs(4),
        )
        .unwrap()
    }
}
impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn literal_regex_glob_and_relative_source_lines() {
    let t = Tree::new();
    t.put("src/ā.rs", "one\nhello.a world\nhelloXa\n");
    t.put("other.txt", "hello.a\n");
    assert_eq!(
        t.search(json!({"query":"hello.a", "glob":"**/*.rs"}), &|_| true),
        "src/ā.rs:2: hello.a world"
    );
    assert_eq!(
        t.search(
            json!({"query":"^hello.a$", "regex":true, "glob":"**/*.rs"}),
            &|_| true
        ),
        "src/ā.rs:3: helloXa"
    );
    assert_eq!(t.search(json!({"glob":"**/*.rs"}), &|_| true), "src/ā.rs");
    assert!(
        run(
            &[],
            &serde_json::from_value(json!({"query":"[", "regex":true})).unwrap(),
            &|_| true,
            &|_, _| true,
            Duration::from_secs(1)
        )
        .is_err()
    );
    assert!(
        run(
            &[],
            &Request::default(),
            &|_| true,
            &|_, _| true,
            Duration::from_secs(1)
        )
        .is_err()
    );
}

#[test]
fn ignores_apply_even_without_a_git_repository() {
    let t = Tree::new();
    t.put(".gitignore", "ignored/\n*.log\n");
    t.put("nested/.gitignore", "private.txt\n");
    for path in [
        "ignored/a.txt",
        "a.log",
        "nested/private.txt",
        ".env",
        "node_modules/a.txt",
    ] {
        t.put(path, "needle");
    }
    t.put("nested/ok.txt", "needle");
    assert_eq!(
        t.search(json!({"query":"needle"}), &|_| true),
        "nested/ok.txt:1: needle"
    );
}

#[test]
fn denied_files_and_folders_never_return_names_or_content() {
    let t = Tree::new();
    for p in [
        "vault/key.txt",
        "app-data/state.txt",
        "denied.txt",
        "ok.txt",
    ] {
        t.put(p, "needle");
    }
    let allowed = |p: &Path| {
        !["vault", "app-data", "denied.txt"]
            .iter()
            .any(|n| p.starts_with(t.0.join(n)))
    };
    assert_eq!(
        t.search(json!({"query":"needle"}), &allowed),
        "ok.txt:1: needle"
    );
    assert_eq!(t.search(json!({"glob":"**/*"}), &allowed), "ok.txt");
    assert_eq!(
        run(
            &[],
            &serde_json::from_value(json!({"glob":"**/*"})).unwrap(),
            &|_| true,
            &|_, _| true,
            Duration::from_secs(1)
        )
        .unwrap(),
        "No hits."
    );
}

#[cfg(any(unix, windows))]
#[test]
fn links_and_linked_directories_never_expose_outside_files() {
    let t = Tree::new();
    let other = Tree::new();
    other.put("outside.txt", "needle");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&other.0, t.0.join("linked")).unwrap();
        std::os::unix::fs::symlink(other.0.join("outside.txt"), t.0.join("file.txt")).unwrap();
    }
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(t.0.join("linked"))
            .arg(&other.0)
            .status()
            .unwrap();
        assert!(status.success());
    }
    assert_eq!(t.search(json!({"glob":"**/*"}), &|_| true), "No hits.");
    assert_eq!(t.search(json!({"query":"needle"}), &|_| true), "No hits.");
}

#[test]
fn result_line_file_size_and_time_limits_are_observable() {
    let t = Tree::new();
    t.put(
        "small.txt",
        &format!("needle{}\nneedle\nneedle", "ā".repeat(2000)),
    );
    t.put("huge.txt", &"needle\n".repeat(200_000));
    let answer = t.search(json!({"query":"needle", "limit":1}), &|_| true);
    assert!(answer.starts_with("small.txt:1: needle"), "{answer}");
    assert!(answer.lines().next().unwrap().chars().count() <= 540);
    assert!(answer.ends_with("2 more hits not shown"), "{answer}");
    let folders = [Folder {
        path: t.0.clone(),
        session_id: None,
    }];
    let answer = run(
        &folders,
        &serde_json::from_value(json!({"query":"needle"})).unwrap(),
        &|_| true,
        &|_, _| true,
        Duration::ZERO,
    )
    .unwrap();
    assert!(answer.contains("Search limit reached."), "{answer}");
    assert!(!answer.contains("small.txt"));
}

#[cfg(unix)]
#[test]
fn replacing_the_search_root_or_a_listed_parent_cannot_redirect_a_read() {
    let t = Tree::new();
    let outside = Tree::new();
    t.put("nested/file.txt", "inside");
    outside.put("file.txt", "outside");
    std::fs::rename(t.0.join("nested"), t.0.join("saved")).unwrap();
    std::os::unix::fs::symlink(&outside.0, t.0.join("nested")).unwrap();
    assert!(crate::search_walk::read(&t.0, "nested/file.txt").is_none());
    let saved = t.0.with_extension("saved");
    std::fs::rename(&t.0, &saved).unwrap();
    std::os::unix::fs::symlink(&outside.0, &t.0).unwrap();
    assert!(crate::search_walk::read(&t.0, "file.txt").is_none());
    std::fs::remove_file(&t.0).unwrap();
    std::fs::rename(saved, &t.0).unwrap();
}

#[test]
fn exceeding_the_ignore_budget_never_exposes_a_file_from_the_unread_rules() {
    let t = Tree::new();
    t.put(
        ".gitignore",
        &format!("{}secret.txt\n", "# comment\n".repeat(4096)),
    );
    t.put("secret.txt", "needle");
    let answer = t.search(json!({"query":"needle"}), &|_| true);
    assert!(!answer.contains("secret.txt"), "{answer}");
    assert!(answer.contains("Search limit reached."), "{answer}");
}
