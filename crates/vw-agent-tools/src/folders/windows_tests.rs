//! Windows folder-guard regressions: drive, verbatim and junction paths are judged as the kernel walks them.
//! Depends on the parent folder module and the local filesystem; called only by the Windows test harness.

use std::path::{Component, Path, PathBuf, Prefix};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::json;

use super::*;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempTree {
    root: PathBuf,
    junctions: Vec<PathBuf>,
}

impl TempTree {
    fn new() -> std::io::Result<Self> {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("vw-folders-{}-{id}", std::process::id()));
        std::fs::create_dir(&root)?;
        Ok(Self {
            root,
            junctions: Vec::new(),
        })
    }

    fn junction(&mut self, link: &Path, target: &Path) -> std::io::Result<()> {
        let output = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(normalize_separators(link))
            .arg(normalize_separators(target))
            .output()?;
        assert!(output.status.success(), "mklink /J failed: {output:?}");
        self.junctions.push(link.to_path_buf());
        Ok(())
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        for link in self.junctions.iter().rev() {
            std::fs::remove_dir(link).unwrap_or_else(|error| {
                panic!("failed to remove junction {}: {error}", link.display());
            });
        }
        std::fs::remove_dir_all(&self.root).unwrap_or_else(|error| {
            panic!(
                "failed to remove temporary tree {}: {error}",
                self.root.display()
            );
        });
    }
}

fn copies() -> std::io::Result<(TempTree, Layout, PathBuf, PathBuf)> {
    let temp = TempTree::new()?;
    let home = temp.root.join("vibe-home");
    let own = home.join("worktrees/repo-a/own");
    let other = home.join("worktrees/repo-b/other");
    std::fs::create_dir_all(&own)?;
    std::fs::create_dir_all(&other)?;
    let layout = Layout::new(&home, &own)?;
    Ok((temp, layout, own, other))
}

#[test]
fn drive_root_and_drive_letter_case_are_stable() -> std::io::Result<()> {
    let (_temp, layout, own, other) = copies()?;
    let root = own
        .ancestors()
        .last()
        .ok_or_else(|| std::io::Error::other("no drive root"))?;
    assert!(root.is_absolute());
    assert_eq!(resolve(root)?, normalize_prefix(root));
    if Path::new(r"C:\").try_exists()? {
        assert_eq!(resolve(Path::new(r"C:\"))?, Path::new(r"\\?\C:\"));
    }
    let other_text = other.to_string_lossy();
    let first = other_text
        .chars()
        .next()
        .ok_or_else(|| std::io::Error::other("empty path"))?;
    let opposite = if first.is_ascii_lowercase() {
        first.to_ascii_uppercase()
    } else {
        first.to_ascii_lowercase()
    };
    let changed_case = format!("{opposite}{}", &other_text[first.len_utf8()..]);
    let refusal = layout.check(Path::new(&changed_case), &own, Access::Read, &|_, _| {
        Err("busy".into())
    });
    assert!(
        refusal.as_deref().is_some_and(|text| text.contains("busy")),
        "{refusal:?}"
    );
    Ok(())
}

#[test]
fn ordinary_verbatim_and_mixed_separators_obey_copy_permits() -> std::io::Result<()> {
    let (_temp, layout, own, other) = copies()?;
    let mixed_own = format!("{}/file.txt", own.display()).replace("\\repo-a\\", "/repo-a\\");
    let mixed_other = format!("{}/file.txt", other.display()).replace("\\repo-b\\", "/repo-b\\");
    let verbatim_other = format!(
        r"\\?\{}\file.txt",
        other.to_string_lossy().replace('/', "\\")
    );
    let verbatim_own = format!(r"\\?\{}\file.txt", own.to_string_lossy().replace('/', "\\"));
    let permits = |copy: &Path, is_own: bool| {
        if is_own {
            assert_eq!(copy, layout.own.as_deref().unwrap_or(Path::new("")));
            Ok(())
        } else {
            Err("other copy busy".to_owned())
        }
    };
    for path in [
        own.join("file.txt"),
        PathBuf::from(mixed_own),
        PathBuf::from(&verbatim_own),
        PathBuf::from(verbatim_own.replace("\\file.txt", "/file.txt")),
    ] {
        assert!(
            layout.check(&path, &own, Access::Write, &permits).is_none(),
            "{path:?}"
        );
    }
    for path in [
        other.join("file.txt"),
        PathBuf::from(mixed_other),
        PathBuf::from(&verbatim_other),
        PathBuf::from(verbatim_other.replace("\\file.txt", "/file.txt")),
    ] {
        let refusal = layout.check(&path, &own, Access::Read, &permits);
        assert!(
            refusal
                .as_deref()
                .is_some_and(|text| text.contains("other copy busy")),
            "{path:?}: {refusal:?}"
        );
    }
    assert_eq!(
        resolve(&other)?,
        resolve(
            Path::new(&verbatim_other)
                .parent()
                .ok_or_else(|| std::io::Error::other("no parent"))?
        )?
    );
    Ok(())
}

#[test]
fn junction_is_followed_before_parent_component() -> std::io::Result<()> {
    let (mut temp, layout, own, other) = copies()?;
    let deep = other.join("deep");
    std::fs::create_dir(&deep)?;
    let link = own.join("link");
    temp.junction(&link, &deep)?;
    let kernel_order = link.join("../escaped.txt");
    assert_eq!(
        resolve(&kernel_order)?,
        resolve(&other.join("escaped.txt"))?
    );
    let refusal = layout.check(&kernel_order, &own, Access::Write, &|_, _| {
        Err("busy".into())
    });
    assert!(
        refusal.as_deref().is_some_and(|text| text.contains("busy")),
        "{refusal:?}"
    );
    Ok(())
}

#[test]
fn verbatim_cwd_keeps_relative_junction_parent_for_both_path_checks() -> std::io::Result<()> {
    let (mut temp, original_layout, own, other) = copies()?;
    let deep = other.join("deep");
    std::fs::create_dir(&deep)?;
    temp.junction(&own.join("link"), &deep)?;
    let verbatim_cwd = std::fs::canonicalize(&own)?;
    let layout = Layout::new(&original_layout.vibe_home, &verbatim_cwd)?;
    let relative = Path::new(r"link\..\escaped.txt");
    let raw = PathBuf::from(format!(
        r"{}\{}",
        verbatim_cwd.display(),
        relative.display()
    ));
    assert!(
        raw.components().any(|part| part.as_os_str() == ".."),
        "the test path lost its parent component: {raw:?}"
    );
    assert_eq!(resolve(&raw)?, resolve(&other.join("escaped.txt"))?);
    let busy = |_: &Path, is_own: bool| {
        assert!(!is_own, "junction traversal reached the own copy");
        Err("other copy busy".to_owned())
    };
    for refusal in [
        layout.check(relative, &layout.cwd, Access::Read, &busy),
        check_tool_input(
            &layout,
            &json!({"file_path": relative.to_string_lossy()}),
            Access::Read,
            &busy,
        ),
    ] {
        assert!(
            refusal
                .as_deref()
                .is_some_and(|text| text.contains("other copy busy")),
            "{refusal:?}"
        );
    }
    Ok(())
}

#[test]
fn ordinary_trailing_dot_or_space_aliases_keep_copy_identity() -> std::io::Result<()> {
    let (_temp, layout, own, other) = copies()?;
    let leaf = "alias-target.txt";
    std::fs::write(own.join(leaf), b"test")?;
    std::fs::write(other.join(leaf), b"test")?;
    let home = own
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or_else(|| std::io::Error::other("no Vibe home"))?;
    for suffix in [".", " "] {
        for (copy, inner, expected_own) in
            [(&own, "repo-a/own", true), (&other, "repo-b/other", false)]
        {
            let alias = home
                .join(format!("worktrees{suffix}"))
                .join(inner)
                .join(leaf);
            match std::fs::canonicalize(&alias) {
                Ok(actual) => {
                    let expected = resolve(&copy.join(leaf))?;
                    assert_eq!(normalize_prefix(&actual), expected, "{alias:?}");
                    assert_eq!(resolve(&alias)?, expected, "{alias:?}");
                    let expected_copy = resolve(copy)?;
                    let refusal = layout.check(&alias, &own, Access::Read, &|checked, is_own| {
                        assert_eq!(checked, expected_copy.as_path(), "{alias:?}");
                        assert_eq!(is_own, expected_own, "{alias:?}");
                        if is_own {
                            Ok(())
                        } else {
                            Err("other copy busy".to_owned())
                        }
                    });
                    if expected_own {
                        assert!(refusal.is_none(), "{alias:?}: {refusal:?}");
                    } else {
                        assert!(
                            refusal
                                .as_deref()
                                .is_some_and(|text| text.contains("other copy busy")),
                            "{alias:?}: {refusal:?}"
                        );
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    eprintln!("ordinary alias is not resolved by std::fs on this host: {alias:?}");
                }
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}

#[test]
fn reported_outside_worktrees_cwd_with_absent_suffix_is_allowed() -> std::io::Result<()> {
    let temp = TempTree::new()?;
    let home = temp.root.join("unrelated-vibe-home");
    std::fs::create_dir(&home)?;
    let cwd = Path::new(r"C:\Users\Someone\Downloads\VibeWork");
    let layout = Layout::new(&home, cwd)?;
    assert!(layout.own.is_none());
    let no_copy_check = |_: &Path, _: bool| -> Result<(), String> {
        panic!("outside-worktrees paths must not ask for a copy permit")
    };
    for access in [Access::Read, Access::Write] {
        assert!(
            check_tool_input(
                &layout,
                &json!({"file_path": "notes.txt"}),
                access,
                &no_copy_check
            )
            .is_none()
        );
    }
    assert!(check_command(&layout, "echo hello", None, &no_copy_check).is_none());
    Ok(())
}

#[test]
fn layout_resolution_error_names_the_bad_path() -> std::io::Result<()> {
    let (_temp, layout, own, _other) = copies()?;
    let bad = own.join("bad\0path");
    let error = Layout::new(&layout.vibe_home, &bad)
        .err()
        .ok_or_else(|| std::io::Error::other("Layout::new unexpectedly accepted an invalid cwd"))?;
    assert!(error.to_string().contains("bad"), "{error}");
    let refusal = layout.check(Path::new("bad\0path"), &own, Access::Read, &|_, _| Ok(()));
    assert!(
        refusal
            .as_deref()
            .is_some_and(|text| text.contains("could not be resolved")),
        "{refusal:?}"
    );
    Ok(())
}

#[test]
fn verbatim_path_field_is_not_cut_at_question_mark() -> std::io::Result<()> {
    let (_temp, layout, _own, other) = copies()?;
    let verbatim = format!(
        r"\\?\{}\file.txt",
        other.to_string_lossy().replace('/', "\\")
    );
    let input = json!({"file_path": verbatim});
    let refusal = check_tool_input(&layout, &input, Access::Read, &|_, _| Err("busy".into()));
    assert!(
        refusal.as_deref().is_some_and(|text| text.contains("busy")),
        "{refusal:?}"
    );
    Ok(())
}

#[test]
fn unc_prefix_and_root_are_kept_as_separate_components() {
    for text in [
        r"\\server\share\folder\..\file",
        r"\\?\UNC\server\share\folder\..\file",
    ] {
        let mut parts = Path::new(text).components();
        assert!(matches!(parts.next(), Some(Component::Prefix(_))), "{text}");
        assert_eq!(parts.next(), Some(Component::RootDir), "{text}");
        assert!(
            parts.any(|part| part.as_os_str().to_string_lossy() == ".."),
            "{text}"
        );
    }
    assert!(
        matches!(Path::new(r"\\server\share").components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::UNC(..)))
    );
    assert!(
        matches!(Path::new(r"\\?\UNC\server\share").components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::VerbatimUNC(..)))
    );
    assert_eq!(
        normalize_prefix(Path::new(r"\\SeRvEr\ShArE\folder")),
        normalize_prefix(Path::new(r"\\?\UNC\server\share\folder"))
    );
    assert_eq!(
        normalize_prefix(Path::new(r"\\SeRvEr\ShArE\folder")),
        Path::new(r"\\?\UNC\server\share\folder")
    );
}
