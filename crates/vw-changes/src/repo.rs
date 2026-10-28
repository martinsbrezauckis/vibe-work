//! Finding the git repository a session folder belongs to, and what the review covers in it.
//!
//! Depends on: `git`, `error`. Called by: `baseline`, `diff`, vw-core-api (folder checks, two sessions on
//! one folder).
//! Invariants:
//! - Read only: `rev-parse` and `config --get` read no index and write nothing.
//! - A folder outside any repository, a bare repository and a `.git` folder itself are "no repository"
//!   (`Ok(None)`); git missing or refusing the folder (another user's repository: "dubious ownership") is an
//!   error the UI shows, never a silent non-git fallback.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::ChangesError;
use crate::git::{Call, Index};

/// Oldest git this crate supports (`add`/`restore` with `--pathspec-from-file`).
pub const MIN_GIT: (u32, u32) = (2, 26);

/// A repository as seen from a session folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    /// The working tree root (`git rev-parse --show-toplevel`).
    pub root: PathBuf,
    /// The git directory (absolute; a linked worktree's own, under the main repository's `.git/worktrees`).
    pub git_dir: PathBuf,
    /// The session folder relative to `root` (`""` when it is the root), `/`-separated. The review covers
    /// this subtree only: an agent working in `repo/app` sees changes under `repo/app`, and a home-folder
    /// repository never gets hashed whole for a session in `~/notes`.
    pub scope: String,
    /// The repository's object id hash (`sha1` or `sha256`), to verify objects we read.
    pub object_format: String,
    /// The repository asks git to hide untracked files (`status.showUntrackedFiles=no`, typical for a
    /// dotfiles repository in the home folder): the review then covers tracked files only.
    pub tracked_only: bool,
}

impl Repo {
    /// The scope as a pathspec (`:(literal)app` or `.` for the root).
    #[must_use]
    pub fn scope_pathspec(&self) -> std::ffi::OsString {
        if self.scope.is_empty() {
            ".".into()
        } else {
            crate::paths::literal(&self.scope)
        }
    }
}

/// Finds the repository of `folder`.
///
/// # Errors
/// `GitMissing` when git cannot run; `Refused` when git refuses the folder (e.g. owned by another user) or
/// is older than [`MIN_GIT`].
pub fn discover(folder: &Path) -> Result<Option<Repo>, ChangesError> {
    if !folder.is_dir() {
        return Ok(None);
    }
    let out = Call::new(folder, Index::User)
        .args([
            "rev-parse",
            "--is-inside-work-tree",
            "--show-toplevel",
            "--absolute-git-dir",
            "--show-prefix",
        ])
        .timeout(Duration::from_secs(10))
        .output()?;
    if !out.ok() {
        let message = out.stderr.to_ascii_lowercase();
        if message.contains("dubious ownership") {
            return Err(ChangesError::Refused(
                "git refuses this folder: it belongs to another user (git's safe.directory check)"
                    .into(),
            ));
        }
        // "not a git repository", "must be run in a work tree" (bare), "inside .git": no repository.
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    if lines.next() != Some("true") {
        return Ok(None);
    }
    let (Some(root), Some(git_dir)) = (lines.next(), lines.next()) else {
        return Err(ChangesError::Parse(
            "rev-parse printed too few lines".into(),
        ));
    };
    let prefix = lines.next().unwrap_or("").trim_end_matches('/').to_owned();
    if let Some(found) = crate::git::version(folder)
        && found < MIN_GIT
    {
        return Err(ChangesError::Refused(format!(
            "git {}.{} is too old for the Changes view (needs {}.{} or newer)",
            found.0, found.1, MIN_GIT.0, MIN_GIT.1
        )));
    }
    let root = PathBuf::from(root);
    // A session folder that the enclosing repository ignores (a scratch folder, a home folder under a
    // dotfiles repository that ignores `*`) is not reviewed with that repository: every file in it would be
    // invisible to `git add`. Say why rather than claiming there is no repository. `check-ignore` only reads.
    if !prefix.is_empty() {
        let ignored = Call::new(&root, Index::User)
            .args(["check-ignore", "-q", "--"])
            .args([format!("{prefix}/")])
            .timeout(Duration::from_secs(10))
            .output()?;
        if ignored.ok() {
            // git prints the top level with `/` on Windows too (`C:/Users/...`); the sentence names the folder the way
            // the rest of the app shows it (rebuilt from its components: `\` on Windows, unchanged elsewhere).
            return Err(ChangesError::Refused(format!(
                "Ignored by the {} repository",
                root.components().collect::<PathBuf>().display()
            )));
        }
    }
    let tracked_only = Call::new(&root, Index::User)
        .args(["config", "--get", "status.showUntrackedFiles"])
        .timeout(Duration::from_secs(10))
        .run()
        .is_ok_and(|v| {
            String::from_utf8_lossy(&v)
                .trim()
                .eq_ignore_ascii_case("no")
        });
    let object_format = Call::new(&root, Index::User)
        .args(["rev-parse", "--show-object-format"])
        .timeout(Duration::from_secs(10))
        .run()
        .map_or_else(
            |_| "sha1".to_owned(),
            |v| String::from_utf8_lossy(&v).trim().to_owned(),
        );
    Ok(Some(Repo {
        root,
        git_dir: PathBuf::from(git_dir),
        scope: prefix,
        object_format,
        tracked_only,
    }))
}

/// The tree of `HEAD`, `None` on an unborn branch (a repository without commits).
///
/// # Errors
/// git failures other than "no HEAD yet".
pub fn head_tree(repo: &Repo, index: &Path) -> Result<Option<String>, ChangesError> {
    let out = Call::new(&repo.root, Index::Ours(index))
        .args(["rev-parse", "--verify", "--quiet", "HEAD^{tree}"])
        .output()?;
    if out.ok() {
        Ok(Some(String::from_utf8_lossy(&out.stdout).trim().to_owned()))
    } else {
        Ok(None)
    }
}
