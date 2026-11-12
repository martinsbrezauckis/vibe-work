//! Windows folder-guard regressions: drive, verbatim and junction paths are judged as the kernel walks them.
//! Depends on the parent folder module and the local filesystem; called only by the Windows test harness.

use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::atomic::{AtomicU64, Ordering};

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

// The agent often writes Git Bash drive paths in its commands; they name the same
// folders as their Windows spelling, also inside quoted code.

// Windows names one folder in many spellings: Python code on Windows usually writes `C:/...`, a path can come in
// another case, and separators can be mixed. Each still names the Vibe home inside quoted code.

// After `cd /c/...` the shell is in that Windows folder, so later relative paths are checked from it: a sibling
// copy reached as `../` stays behind its permit.
