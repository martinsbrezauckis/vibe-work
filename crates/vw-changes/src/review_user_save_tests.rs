//! Unit tests of `record_user_save` / `undo_user_save` and `repository_relative` (review.rs): the user's saved
//! lines survive a revert of the agent's change, in every spelling of the repository's folder.

use super::*;
use std::process::Command;

#[test]
fn disjoint_user_edit_survives_agent_file_revert() {
    let root = std::env::temp_dir().join(format!(
        "vw-save-review-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let git = Command::new("git")
        .arg("init")
        .arg("-q")
        .arg(&root)
        .status()
        .unwrap();
    assert!(git.success());
    let file = root.join("note.txt");
    let original = b"agent line\nunchanged 1\nunchanged 2\nuser line\n";
    let agent = b"agent edit\nunchanged 1\nunchanged 2\nuser line\n";
    let saved = b"agent edit\nunchanged 1\nunchanged 2\nuser edit\n";
    std::fs::write(&file, original).unwrap();
    let changes = root.join("review-data");
    crate::store::prepare_root(&changes).unwrap();
    let repo = crate::repo::discover(&root).unwrap().unwrap();
    let dir = crate::store::SessionDir::open(&changes, "session").unwrap();
    let mut baseline = Baseline::take(repo, dir).unwrap();
    std::fs::write(&file, agent).unwrap();
    std::fs::write(&file, saved).unwrap();
    let _ = record_user_save(&mut baseline, &file, agent, saved).unwrap();
    let repo = baseline.repo().clone();
    let dir = baseline.dir().clone();
    drop(baseline);
    let baseline = Baseline::load(repo, dir).unwrap();
    let diff = refresh(&baseline).unwrap();
    let change = diff.file("note.txt").unwrap();
    assert_eq!(change.added, 1);
    assert_eq!(change.removed, 1);
    revert(
        &baseline,
        &Target::File {
            path: "note.txt".into(),
            blob: change.blob.clone(),
        },
        &[],
    )
    .unwrap();
    assert_eq!(
        std::fs::read(&file).unwrap(),
        b"agent line\nunchanged 1\nunchanged 2\nuser edit\n"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn overlapping_user_edit_becomes_protected_baseline() {
    let base = b"one\ntwo\n";
    let agent = b"agent\ntwo\n";
    let saved = b"user\ntwo\n";
    assert_eq!(merge_user_save(base, agent, saved), None);

    let root = std::env::temp_dir().join(format!(
        "vw-overlap-review-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    assert!(
        Command::new("git")
            .arg("init")
            .arg("-q")
            .arg(&root)
            .status()
            .unwrap()
            .success()
    );
    let file = root.join("note.txt");
    std::fs::write(&file, base).unwrap();
    let changes = root.join("review-data");
    crate::store::prepare_root(&changes).unwrap();
    let repo = crate::repo::discover(&root).unwrap().unwrap();
    let dir = crate::store::SessionDir::open(&changes, "session").unwrap();
    let mut baseline = Baseline::take(repo, dir).unwrap();
    std::fs::write(&file, agent).unwrap();
    let before_view = refresh(&baseline).unwrap();
    let old_target = Target::File {
        path: "note.txt".into(),
        blob: before_view.file("note.txt").unwrap().blob.clone(),
    };
    std::fs::write(&file, saved).unwrap();
    let _ = record_user_save(&mut baseline, &file, agent, saved).unwrap();
    let repo = baseline.repo().clone();
    let dir = baseline.dir().clone();
    drop(baseline);
    let baseline = Baseline::load(repo, dir).unwrap();
    assert!(refresh(&baseline).unwrap().file("note.txt").is_none());
    assert!(matches!(
        revert(&baseline, &old_target, &[]),
        Err(ChangesError::Stale { .. })
    ));
    assert_eq!(std::fs::read(&file).unwrap(), saved);
    std::fs::remove_dir_all(root).unwrap();
}

/// A repository reached through a symlinked folder (`link -> real`), the way a macOS temporary folder
/// (`/var` -> `/private/var`) is: git answers the root as `real`, the editor names the file through `link`.
#[cfg(unix)]
struct LinkedRepo {
    base: std::path::PathBuf,
    link: std::path::PathBuf,
}

#[cfg(unix)]
impl LinkedRepo {
    fn new(name: &str) -> Self {
        let base = std::env::temp_dir().join(format!(
            "vw-linked-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let real = base.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = base.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(
            Command::new("git")
                .arg("init")
                .arg("-q")
                .arg(&real)
                .status()
                .unwrap()
                .success()
        );
        Self { base, link }
    }

    /// A baseline for the review of `folder` (below `link`), stored outside the working tree.
    fn baseline(&self, folder: &Path) -> Baseline {
        let changes = self.base.join("review-data");
        crate::store::prepare_root(&changes).unwrap();
        let repo = crate::repo::discover(folder).unwrap().unwrap();
        let dir = crate::store::SessionDir::open(&changes, "session").unwrap();
        Baseline::take(repo, dir).unwrap()
    }
}

#[cfg(unix)]
impl Drop for LinkedRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

/// The editor names the file through the linked folder while git names the root by its real folder: the
/// user's line must still reach the baseline, so reverting the agent's file change keeps it.
#[cfg(unix)]
#[test]
fn save_through_a_linked_folder_survives_agent_file_revert() {
    let repo = LinkedRepo::new("save");
    let file = repo.link.join("note.txt");
    let original = b"agent line\nunchanged 1\nunchanged 2\nuser line\n";
    let agent = b"agent edit\nunchanged 1\nunchanged 2\nuser line\n";
    let saved = b"agent edit\nunchanged 1\nunchanged 2\nuser edit\n";
    std::fs::write(&file, original).unwrap();
    let mut baseline = repo.baseline(&repo.link);
    assert_ne!(
        baseline.repo().root,
        repo.link,
        "git names the root by its real folder"
    );
    std::fs::write(&file, agent).unwrap();
    std::fs::write(&file, saved).unwrap();
    let recorded = record_user_save(&mut baseline, &file, agent, saved).unwrap();
    let diff = refresh(&baseline).unwrap();
    let change = diff.file("note.txt").unwrap();
    let counts = (change.added, change.removed);
    revert(
        &baseline,
        &Target::File {
            path: "note.txt".into(),
            blob: change.blob.clone(),
        },
        &[],
    )
    .unwrap();
    // The user-visible result first: the revert must not take the user's saved line away.
    assert_eq!(
        String::from_utf8_lossy(&std::fs::read(&file).unwrap()),
        "agent line\nunchanged 1\nunchanged 2\nuser edit\n"
    );
    assert!(recorded.is_some(), "the save was not recorded");
    assert_eq!(
        counts,
        (1, 1),
        "only the agent's line is offered for revert"
    );
}

/// The overlapping case through a linked folder: the saved file becomes the baseline, so the agent's
/// change is no longer offered for revert and the user's bytes stay.
#[cfg(unix)]
#[test]
fn overlapping_save_through_a_linked_folder_becomes_protected_baseline() {
    let repo = LinkedRepo::new("overlap");
    let file = repo.link.join("note.txt");
    std::fs::write(&file, b"one\ntwo\n").unwrap();
    let mut baseline = repo.baseline(&repo.link);
    std::fs::write(&file, b"agent\ntwo\n").unwrap();
    let shown = refresh(&baseline).unwrap();
    let old_target = Target::File {
        path: "note.txt".into(),
        blob: shown.file("note.txt").unwrap().blob.clone(),
    };
    std::fs::write(&file, b"user\ntwo\n").unwrap();
    let recorded = record_user_save(&mut baseline, &file, b"agent\ntwo\n", b"user\ntwo\n").unwrap();
    assert!(recorded.is_some(), "the save was not recorded");
    assert!(refresh(&baseline).unwrap().file("note.txt").is_none());
    assert!(matches!(
        revert(&baseline, &old_target, &[]),
        Err(ChangesError::Stale { .. })
    ));
    assert_eq!(std::fs::read(&file).unwrap(), b"user\ntwo\n");
}

/// Genuinely outside files stay unrecorded in every spelling: a sibling of the repository reached through
/// the same link's parent, and a file outside the session's scope (the review covers `link/app` only).
#[cfg(unix)]
#[test]
fn saves_outside_the_repository_or_scope_are_not_recorded() {
    let repo = LinkedRepo::new("outside");
    let app = repo.link.join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("in.txt"), b"a\n").unwrap();
    std::fs::write(repo.link.join("other.txt"), b"a\n").unwrap();
    let outside = repo.base.join("elsewhere.txt");
    std::fs::write(&outside, b"a\n").unwrap();
    let mut baseline = repo.baseline(&app);
    assert_eq!(baseline.repo().scope, "app");
    assert!(
        record_user_save(&mut baseline, &outside, b"a\n", b"b\n")
            .unwrap()
            .is_none()
    );
    assert!(
        record_user_save(&mut baseline, &repo.link.join("other.txt"), b"a\n", b"b\n")
            .unwrap()
            .is_none()
    );
    // The control: inside the scope through the link is recorded.
    std::fs::write(app.join("in.txt"), b"b\n").unwrap();
    assert!(
        record_user_save(&mut baseline, &app.join("in.txt"), b"a\n", b"b\n")
            .unwrap()
            .is_some()
    );
}

/// The comparison itself: one canonical form for both sides, the file name joined back (a new file, a
/// file that is a symlink stays the link), and paths compared as written when the root is gone.
#[cfg(unix)]
#[test]
fn repository_relative_compares_canonical_forms() {
    let repo = LinkedRepo::new("relative");
    let real = repo.base.join("real");
    assert_eq!(
        repository_relative(&real, &repo.link.join("new.txt")),
        Some(PathBuf::from("new.txt"))
    );
    assert_eq!(
        repository_relative(&repo.link, &real.join("b.txt")),
        Some(PathBuf::from("b.txt"))
    );
    let outside = repo.base.join("out.txt");
    std::fs::write(&outside, b"x").unwrap();
    std::os::unix::fs::symlink(&outside, real.join("to-out")).unwrap();
    assert_eq!(
        repository_relative(&real, &repo.link.join("to-out")),
        Some(PathBuf::from("to-out"))
    );
    assert_eq!(repository_relative(&real, &outside), None);
    assert_eq!(
        repository_relative(&real, &repo.base.join("real-sibling/x")),
        None
    );
    let gone = repo.base.join("gone");
    assert_eq!(
        repository_relative(&gone, &gone.join("x.txt")),
        Some(PathBuf::from("x.txt"))
    );
}

/// Windows spellings of one folder: the verbatim form `canonicalize` answers (`\\?\C:\...`, what the File tab
/// holds), the plain form, `/` separators, another letter case, and the temporary folder as the environment
/// names it (a short name such as `RUNNER~1` on CI runners). Every pairing of root and file spelling must place
/// the file at `note.txt`; a file outside stays outside.
#[cfg(windows)]
#[test]
fn repository_relative_matches_windows_spellings() {
    let base = std::env::temp_dir().join(format!(
        "vw-win-spell-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let as_written = base.join("repo");
    std::fs::create_dir_all(&as_written).unwrap();
    std::fs::write(as_written.join("note.txt"), b"x").unwrap();
    let verbatim = as_written.canonicalize().unwrap();
    assert!(verbatim.to_string_lossy().starts_with(r"\\?\"));
    let plain = vw_platform::plain_path(&verbatim);
    let slashes = PathBuf::from(plain.to_string_lossy().replace('\\', "/"));
    let upper = PathBuf::from(plain.to_string_lossy().to_uppercase());
    let lower = PathBuf::from(plain.to_string_lossy().to_lowercase());
    let spellings = [&as_written, &verbatim, &plain, &slashes, &upper, &lower];
    for root in spellings {
        for folder in spellings {
            assert_eq!(
                repository_relative(root, &folder.join("note.txt")),
                Some(PathBuf::from("note.txt")),
                "root {} file in {}",
                root.display(),
                folder.display()
            );
        }
        assert_eq!(repository_relative(root, &base.join("outside.txt")), None);
    }
    std::fs::remove_dir_all(base).unwrap();
}

/// The editor's path as the File tab holds it (verbatim, from `canonicalize`) against git's root (`C:/...`):
/// the save is recorded and a revert of the agent's line keeps the user's line.
#[cfg(windows)]
#[test]
fn verbatim_editor_path_save_survives_agent_file_revert() {
    let root = std::env::temp_dir().join(format!(
        "vw-win-save-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    assert!(
        Command::new("git")
            .arg("init")
            .arg("-q")
            .arg(&root)
            .status()
            .unwrap()
            .success()
    );
    let original = b"agent line\nunchanged 1\nunchanged 2\nuser line\n";
    let agent = b"agent edit\nunchanged 1\nunchanged 2\nuser line\n";
    let saved = b"agent edit\nunchanged 1\nunchanged 2\nuser edit\n";
    std::fs::write(root.join("note.txt"), original).unwrap();
    let changes = root.join("review-data");
    crate::store::prepare_root(&changes).unwrap();
    let repo = crate::repo::discover(&root).unwrap().unwrap();
    let dir = crate::store::SessionDir::open(&changes, "session").unwrap();
    let mut baseline = Baseline::take(repo, dir).unwrap();
    let file = root.join("note.txt").canonicalize().unwrap();
    std::fs::write(&file, agent).unwrap();
    std::fs::write(&file, saved).unwrap();
    let recorded = record_user_save(&mut baseline, &file, agent, saved).unwrap();
    let diff = refresh(&baseline).unwrap();
    let change = diff.file("note.txt").unwrap();
    revert(
        &baseline,
        &Target::File {
            path: "note.txt".into(),
            blob: change.blob.clone(),
        },
        &[],
    )
    .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&std::fs::read(&file).unwrap()),
        "agent line\nunchanged 1\nunchanged 2\nuser edit\n",
        "root {} file {}",
        baseline.repo().root.display(),
        file.display()
    );
    assert!(recorded.is_some(), "the save was not recorded");
    drop(baseline);
    std::fs::remove_dir_all(root).unwrap();
}
