//! Windows spellings of a project folder against the pointer files git itself writes (`C:/...`, long names, the
//! on-disk case, whatever spelling the folder had when git ran). Depends on a `git` on PATH; part of browser.rs.

use super::{Limits, discover, revalidate, safe_path};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} in {}", dir.display());
}

/// The 8.3 spelling of an existing folder (cmd's `%~s` expansion), when the volume keeps short names.
fn short_name(path: &Path) -> Option<PathBuf> {
    let output = Command::new("cmd")
        .raw_arg(format!(
            "/d /c for %I in (\"{}\") do @echo %~sI",
            path.display()
        ))
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let short = PathBuf::from(text.trim());
    (output.status.success() && short.is_dir()).then_some(short)
}

/// A linked copy made by `git worktree add` inside the project is found once, as an in-project copy that is not
/// restricted and passes revalidation, whether the project folder is written in its everyday spelling, in upper or
/// lower case, or with its short name.
#[test]
fn a_linked_copy_git_made_is_one_in_project_copy_in_every_project_spelling() {
    let tick = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base = std::env::temp_dir().join(format!(
        "vw-git-browser-spellings-{}-{tick}",
        std::process::id()
    ));
    let as_written = base.join("Project Folder");
    let main = as_written.join("main");
    std::fs::create_dir_all(&main).unwrap();
    git(&main, &["init", "-q"]);
    git(
        &main,
        &[
            "-c",
            "user.name=check",
            "-c",
            "user.email=check@example.invalid",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "start",
        ],
    );
    git(&main, &["worktree", "add", "-q", "../copy"]);
    let pointer = std::fs::read_to_string(as_written.join("copy").join(".git")).unwrap();
    let plain = vw_platform::plain_path(&as_written.canonicalize().unwrap());
    let mut spellings = vec![
        plain.clone(),
        PathBuf::from(plain.to_string_lossy().to_uppercase()),
        PathBuf::from(plain.to_string_lossy().to_lowercase()),
    ];
    match short_name(&plain) {
        Some(short) if short != plain => spellings.push(short),
        // Not a pass for the short spelling: this volume makes no 8.3 names, so that spelling is not checked here.
        _ => eprintln!("no short name on this volume; the short spelling is not checked"),
    }
    for project in &spellings {
        let found = discover(
            project,
            Limits {
                depth: 4,
                entries: 1_000,
                time: Duration::from_secs(5),
            },
        );
        let copies: Vec<_> = found.nodes.iter().filter(|n| n.kind == "copy").collect();
        assert_eq!(
            copies.len(),
            1,
            "project {} pointer {pointer:?}: {found:?}",
            project.display()
        );
        let copy = copies[0];
        assert!(
            !copy.outside && copy.restricted.is_none(),
            "project {} pointer {pointer:?}: {copy:?}",
            project.display()
        );
        assert_eq!(copy.path, project.join("copy"));
        assert!(revalidate(copy, project), "{copy:?}");
        let repository = found.nodes.iter().find(|n| n.kind == "repository").unwrap();
        assert!(repository.restricted.is_none(), "{repository:?}");
    }
    std::fs::remove_dir_all(base).unwrap();
}

/// A pointer to a repository outside the project stays outside in every spelling: the respelling only renames a
/// folder that is inside.
#[test]
fn a_pointer_outside_the_project_stays_restricted_in_every_spelling() {
    let tick = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base = std::env::temp_dir().join(format!(
        "vw-git-browser-outside-{}-{tick}",
        std::process::id()
    ));
    let project = base.join("project");
    let outside = base.join("outside");
    std::fs::create_dir_all(project.join("bad")).unwrap();
    std::fs::create_dir_all(outside.join(".git")).unwrap();
    let real_outside = vw_platform::plain_path(&outside.canonicalize().unwrap());
    std::fs::write(
        project.join("bad").join(".git"),
        format!(
            "gitdir: {}\n",
            real_outside
                .join(".git")
                .to_string_lossy()
                .replace('\\', "/")
        ),
    )
    .unwrap();
    let plain = vw_platform::plain_path(&project.canonicalize().unwrap());
    for spelling in [
        plain.clone(),
        PathBuf::from(plain.to_string_lossy().to_lowercase()),
    ] {
        let found = discover(
            &spelling,
            Limits {
                depth: 4,
                entries: 1_000,
                time: Duration::from_secs(5),
            },
        );
        let bad = found
            .nodes
            .iter()
            .find(|n| n.path.ends_with("bad"))
            .unwrap();
        assert!(bad.restricted.is_some(), "{bad:?}");
    }
    std::fs::remove_dir_all(base).unwrap();
}

/// A folder in the `\\?\C:\...` spelling `canonicalize` gives passes the link check exactly as its plain spelling
/// does, drive roots included, and the project is discovered in it. A junction inside it is still refused in both
/// spellings, and `..` hops above the drive root never land inside the folder.
#[test]
fn the_canonical_spelling_passes_the_link_check_and_a_junction_inside_it_does_not() {
    let tick = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base = std::env::temp_dir().join(format!(
        "vw-git-browser-verbatim-{}-{tick}",
        std::process::id()
    ));
    let project = base.join("project");
    let outside = base.join("outside");
    std::fs::create_dir_all(project.join("inner").join(".git")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let status = Command::new("cmd")
        .raw_arg(format!(
            "/d /c mklink /J \"{}\" \"{}\"",
            project.join("link").display(),
            outside.display()
        ))
        .output()
        .unwrap();
    assert!(status.status.success(), "{status:?}");
    let canonical = project.canonicalize().unwrap();
    assert!(
        canonical.to_string_lossy().starts_with(r"\\?\"),
        "{canonical:?}"
    );
    let plain = vw_platform::plain_path(&canonical);
    for root in [&canonical, &plain] {
        let drive_root: PathBuf = root.components().take(2).collect();
        assert_eq!(
            safe_path(Path::new(""), &drive_root),
            Some(drive_root.clone())
        );
        assert_eq!(safe_path(Path::new(""), root).as_ref(), Some(root));
        assert_eq!(
            safe_path(root, Path::new("inner")),
            Some(root.join("inner"))
        );
        assert_eq!(
            safe_path(Path::new(""), &root.join("link")),
            None,
            "{root:?}"
        );
        assert_eq!(
            safe_path(Path::new(""), &root.join("link").join("x")),
            None,
            "{root:?}"
        );
        assert_eq!(safe_path(root, Path::new("link")), None, "{root:?}");
        // Hops above the drive root never land inside the folder: the plain spelling refuses them, and joining
        // them to a `\\?\` folder already stops at the drive root (`PathBuf::push` resolves `..` there).
        let above = "..\\".repeat(root.components().count() + 1);
        assert!(
            safe_path(root, Path::new(&above)).is_none_or(|p| !p.starts_with(root)),
            "{root:?}"
        );
        let found = discover(
            root,
            Limits {
                depth: 4,
                entries: 1_000,
                time: Duration::from_secs(5),
            },
        );
        assert!(
            found
                .nodes
                .iter()
                .any(|n| n.path == root.join("inner") && n.restricted.is_none()),
            "{root:?}: {found:?}"
        );
        assert!(
            !found
                .directories
                .iter()
                .any(|d| d.starts_with(root.join("link"))),
            "{root:?}: the junction was walked: {found:?}"
        );
    }
    std::fs::remove_dir(project.join("link")).unwrap();
    std::fs::remove_dir_all(base).unwrap();
}
