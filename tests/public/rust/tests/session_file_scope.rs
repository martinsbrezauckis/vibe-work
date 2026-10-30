//! File access is limited to a session folder and approved paths.

use std::cell::RefCell;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use vw_core_api::files_scope::{RealFs, Scope, ScopeFs, resolve_in};

/// Counts (and records) every filesystem call.
#[derive(Default)]
struct Counting {
    calls: RefCell<Vec<PathBuf>>,
}

impl ScopeFs for Counting {
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        self.calls.borrow_mut().push(path.to_path_buf());
        RealFs.canonicalize(path)
    }
    fn facts(&self, path: &Path) -> io::Result<(bool, u64, Option<SystemTime>)> {
        self.calls.borrow_mut().push(path.to_path_buf());
        RealFs.facts(path)
    }
}

fn scratch(name: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("vw-file-scope");
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = base.join(format!("{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Keep the temporary root in plain path form so file URLs match user-visible paths.
    vw_platform::canonical_plain(&dir).unwrap()
}

/// `file://` URL of an absolute path the way a browser or an agent writes it (`file:///C:/x` on Windows).
fn file_url(path: &Path) -> String {
    let text = path.display().to_string().replace(' ', "%20");
    if cfg!(windows) {
        format!("file:///{}", text.replace('\\', "/"))
    } else {
        format!("file://{text}")
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

struct Fixture {
    session: PathBuf,
    picked: PathBuf,
    outside: PathBuf,
}

fn fixture() -> Fixture {
    let root = scratch("fx");
    let session = root.join("project");
    write(&session.join("src/main.rs"), "fn main() {}\n");
    write(&session.join("My Docs/report.md"), "# Report\n");
    write(&session.join(".env"), "SECRET=1\n");
    write(&session.join("home/notes.txt"), "notes\n");
    std::fs::create_dir_all(session.join("empty-dir")).unwrap();
    let outside = root.join("outside.txt");
    write(&outside, "outside\n");
    let picked = root.join("picked");
    write(&picked.join("design.md"), "# Design\n");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, session.join("link-out.txt")).unwrap();
        std::os::unix::fs::symlink(session.join("src/main.rs"), session.join("link-in.rs"))
            .unwrap();
        std::os::unix::fs::symlink(&session, root.join("project-link")).unwrap();
    }
    Fixture {
        session,
        picked,
        outside,
    }
}

fn window(fx: &Fixture) -> Scope {
    Scope {
        session: fx.session.clone(),
        cwd: None,
        picked: vec![fx.picked.clone()],
        home: Some(fx.session.join("home")),
        hidden: true,
    }
}

fn debug_api(fx: &Fixture) -> Scope {
    Scope {
        picked: Vec::new(),
        hidden: false,
        ..window(fx)
    }
}

#[test]
fn paths_inside_the_session_folder_resolve() {
    let fx = fixture();
    let scope = window(&fx);
    let main = fx.session.join("src/main.rs");
    let abs = main.display().to_string();
    let url = file_url(&main);
    for text in [
        "src/main.rs",
        "./src/main.rs",
        "src/../src/main.rs",
        "  src/main.rs  ",
        abs.as_str(),
        url.as_str(),
    ] {
        let hit =
            resolve_in(&scope, text, &RealFs).unwrap_or_else(|| panic!("{text} should resolve"));
        // `canonical` is the core's own form (verbatim on Windows); compare the everyday spelling.
        assert_eq!(vw_platform::plain_path(&hit.canonical), main, "{text}");
        assert!(hit.in_session);
        assert_eq!(hit.size, 13);
    }
    let spaced = resolve_in(&scope, "My Docs/report.md", &RealFs).unwrap();
    assert!(spaced.canonical.ends_with("My Docs/report.md"));
    let encoded = file_url(&fx.session.join("My Docs").join("report.md"));
    assert!(resolve_in(&scope, &encoded, &RealFs).is_some());
    let home = resolve_in(&scope, "~/notes.txt", &RealFs).unwrap();
    assert!(home.canonical.ends_with("home/notes.txt"));
    #[cfg(unix)]
    {
        let link = resolve_in(&scope, "link-in.rs", &RealFs).unwrap();
        assert_eq!(
            link.canonical, main,
            "a symlink that stays inside is followed"
        );
        let through = Scope {
            session: fx.session.parent().unwrap().join("project-link"),
            ..window(&fx)
        };
        let hit = resolve_in(&through, "src/main.rs", &RealFs).unwrap();
        assert_eq!(hit.canonical, main);
        assert!(hit.in_session);
    }
}

#[test]
fn paths_outside_are_refused_without_touching_the_disk() {
    let fx = fixture();
    let scope = window(&fx);
    let escape = format!("{}/../../etc/passwd", fx.session.display());
    let outside = fx.outside.display().to_string();
    let long = "a".repeat(5000);
    for text in [
        "../outside.txt",
        outside.as_str(),
        "/etc/hostname",
        "/etc/passwd",
        "src/../../outside.txt",
        escape.as_str(),
        "file://server/share/x.txt",
        "file:///etc/passwd",
        "\\\\server\\share\\x.txt",
        "//server/share/x.txt",
        "C:\\Users\\x\\a.txt",
        "C:/Users/x/a.txt",
        "~root/.bashrc",
        "~",
        "src/main.rs\u{0007}",
        "src/\u{0000}main.rs",
        "",
        long.as_str(),
        "/../../../../etc/passwd",
    ] {
        let fs = Counting::default();
        assert!(
            resolve_in(&scope, text, &fs).is_none(),
            "{text:?} must be refused"
        );
        // The only access allowed is the session's own folder (canonicalized once, the core's record).
        assert!(
            fs.calls.borrow().iter().all(|p| p == &fx.session),
            "{text:?} touched the disk: {:?}",
            fs.calls.borrow()
        );
    }
}

#[test]
fn links_out_directories_and_missing_files_give_the_same_refusal() {
    let fx = fixture();
    let scope = window(&fx);
    for text in ["empty-dir", "src", "missing.txt", "src/nope.rs"] {
        assert!(resolve_in(&scope, text, &RealFs).is_none(), "{text}");
    }
    #[cfg(unix)]
    assert!(
        resolve_in(&scope, "link-out.txt", &RealFs).is_none(),
        "a symlink leaving the folder is refused"
    );
}

#[test]
fn hidden_files_and_picked_folders_depend_on_the_origin() {
    let fx = fixture();
    let design = fx.picked.join("design.md").display().to_string();
    let win = resolve_in(&window(&fx), ".env", &RealFs).unwrap();
    assert!(win.in_session);
    let picked = resolve_in(&window(&fx), &design, &RealFs).unwrap();
    assert!(!picked.in_session);
    assert_eq!(picked.root, fx.picked);
    for text in [".env", "./.env", design.as_str()] {
        let fs = Counting::default();
        assert!(resolve_in(&debug_api(&fx), text, &fs).is_none(), "{text}");
        assert!(
            fs.calls.borrow().iter().all(|p| p == &fx.session),
            "{text} touched the disk"
        );
    }
    // A visible file is still fine for the Debug API.
    assert!(resolve_in(&debug_api(&fx), "src/main.rs", &RealFs).is_some());
}

/// A linked session folder accepts canonical paths while keeping lexical access scoped.
#[cfg(unix)]
#[test]
fn a_session_folder_recorded_through_a_symlink_keeps_working_without_widening_the_scope() {
    use vw_core_api::files_scope::recheck_in;
    let fx = fixture();
    let link = fx.session.parent().unwrap().join("project-link");
    let scope = Scope {
        session: link.clone(),
        ..window(&fx)
    };
    let canonical = fx.session.join("src/main.rs");
    let text = canonical.display().to_string();
    let hit =
        resolve_in(&scope, &text, &RealFs).expect("the canonical path of a session file resolves");
    assert!(hit.in_session);
    assert_eq!(
        recheck_in(&scope, &canonical, &RealFs).map(|h| h.canonical),
        Some(canonical)
    );
    for text in ["/etc/hostname", "../outside.txt"] {
        let fs = Counting::default();
        assert!(resolve_in(&scope, text, &fs).is_none());
        let calls = fs.calls.borrow();
        assert!(
            calls.iter().all(|p| p == &link),
            "{text}: only the session folder itself may be touched, saw {calls:?}"
        );
    }
    // A handle's path outside every folder fails the recheck without touching it.
    let fs = Counting::default();
    assert!(recheck_in(&scope, &fx.outside, &fs).is_none());
    assert!(fs.calls.borrow().iter().all(|p| p == &link));
}

/// File names whose 7th byte lies INSIDE a multi-byte character (Latvian, Cyrillic, emoji, a mixed ASCII prefix).
/// The `file://` check compared the first 7 bytes through a `&str` slice and panicked on every one of them.
const SEVENTH_BYTE_INSIDE_A_CHARACTER: [&str; 6] = [
    "āāāā.rs",
    "šķūņu-saraksts.md",
    "Привет.txt",
    "🙂🙂.txt",
    "abcdef🙂.png",
    "123456ā-test",
];

/// A path in agent text is untrusted: a name that is not ASCII must give the ordinary answer (here: the file inside
/// the session folder, or the same refusal when it is missing), never a panic, with or without a `file://` start.
#[test]
fn non_ascii_names_of_any_length_never_panic_the_file_scheme_check() {
    let fx = fixture();
    let scope = window(&fx);
    for name in SEVENTH_BYTE_INSIDE_A_CHARACTER {
        assert!(
            !name.is_char_boundary(7),
            "{name}: the test needs byte 7 inside a character"
        );
        // Missing file: the usual refusal, and the lexical form is the session folder plus the name.
        assert_eq!(
            vw_core_api::files_scope::candidate(&scope, name),
            Some(fx.session.join(name)),
            "{name}"
        );
        assert!(resolve_in(&scope, name, &RealFs).is_none(), "{name}");
        // The same file once it exists.
        write(&fx.session.join(name), "text\n");
        let hit = resolve_in(&scope, name, &RealFs).unwrap_or_else(|| panic!("{name} resolves"));
        assert!(hit.in_session, "{name}");
        assert!(hit.canonical.ends_with(name), "{name}");
        // An absolute path and a `file://` URL ending in the same name.
        let abs = fx.session.join(name);
        assert!(
            resolve_in(&scope, &abs.display().to_string(), &RealFs).is_some(),
            "{name}"
        );
        assert!(
            resolve_in(&scope, &file_url(&abs), &RealFs).is_some(),
            "{name}"
        );
    }
}

/// The scheme is still recognised in any letter case (ASCII input behaves exactly as before the byte comparison),
/// and a URL with a host part is still refused, with a non-ASCII host as well.
#[test]
fn the_file_scheme_is_recognised_in_any_letter_case() {
    let fx = fixture();
    let scope = window(&fx);
    let main = fx.session.join("src/main.rs");
    let lower = file_url(&main);
    let upper = lower.replacen("file", "FILE", 1);
    for url in [lower.as_str(), upper.as_str()] {
        let hit = resolve_in(&scope, url, &RealFs).unwrap_or_else(|| panic!("{url} resolves"));
        assert_eq!(vw_platform::plain_path(&hit.canonical), main, "{url}");
    }
    for url in ["file://server/share/x.txt", "file://сервер/ā.txt"] {
        assert!(resolve_in(&scope, url, &RealFs).is_none(), "{url}");
    }
}

/// The folder the agent works in names a relative path first, then the session folder, and grants nothing: a
/// working folder outside the session folder (or a relative path that leaves it) opens nothing more than before.
#[test]
fn a_working_folder_is_tried_first_and_widens_nothing() {
    let fx = fixture();
    let sub = fx.session.join("web");
    write(&sub.join("src/main.rs"), "fn web() {}\n");
    let working = Scope {
        cwd: Some(sub.clone()),
        ..window(&fx)
    };
    let own = resolve_in(&working, "src/main.rs", &RealFs).unwrap();
    assert_eq!(
        vw_platform::plain_path(&own.canonical),
        sub.join("src/main.rs")
    );
    let main = resolve_in(&working, "My Docs/report.md", &RealFs).unwrap();
    assert_eq!(
        vw_platform::plain_path(&main.canonical),
        fx.session.join("My Docs/report.md")
    );
    assert!(resolve_in(&working, "../../outside.txt", &RealFs).is_none());
    // A working folder outside every allowed folder: its files stay refused, without being touched.
    let elsewhere = Scope {
        cwd: Some(fx.outside.parent().unwrap().to_path_buf()),
        ..window(&fx)
    };
    let counting = Counting::default();
    assert!(resolve_in(&elsewhere, "outside.txt", &counting).is_none());
    // Only the session folder and its own `outside.txt` (the second candidate, as before) are touched.
    assert!(
        !counting.calls.borrow().contains(&fx.outside),
        "{:?}",
        counting.calls.borrow()
    );
}
