//! Folders without git: whether a folder looks like code (the only case where
//! version history is offered), the per-folder "don't ask again", and the one user-confirmed action that
//! writes git state into a folder: `git init` plus a first commit.
//!
//! Depends on: `git`, `store`, `worktree` (size limit), `serde_json`. Called by: vw-core-api.
//! Invariants:
//! - Nothing here runs on its own: `init_repository` is called only for a request that carries the user's
//!   confirmation, and refuses a folder that already has `.git`.
//! - The first commit runs without hooks (`core.hooksPath` pinned, `--no-verify`) and unsigned; untracked
//!   files over the large-file limit are left out and reported.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::ChangesError;
use crate::git::{Call, Index};
use crate::worktree::LARGE_UNTRACKED;

/// Files whose presence at the folder's top level marks a code project.
pub const MANIFESTS: &[&str] = &[
    "package.json",
    "Cargo.toml",
    "pyproject.toml",
    "setup.py",
    "requirements.txt",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "Gemfile",
    "composer.json",
    "CMakeLists.txt",
    "Makefile",
    "deno.json",
    "pubspec.yaml",
    "mix.exs",
];

/// Extensions of source code files.
pub const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "java", "kt", "kts", "swift", "c",
    "h", "cc", "cpp", "hpp", "cs", "rb", "php", "scala", "dart", "lua", "sh", "ps1", "vue",
    "svelte", "ex", "exs", "zig", "sql", "css", "scss", "html",
];

/// Generated folders that a first commit would include when no `.gitignore` excludes them.
const GENERATED: &[&str] = &[
    "node_modules",
    "target",
    ".venv",
    "venv",
    "dist",
    "build",
    "__pycache__",
    ".next",
];

/// True when `folder` looks like code: a manifest at its top, a `*.csproj`/`*.sln` there, or a changed file
/// with a source extension.
#[must_use]
pub fn looks_like_code(folder: &Path, changed: &[String]) -> bool {
    if MANIFESTS.iter().any(|m| folder.join(m).is_file()) {
        return true;
    }
    let top_level_project = std::fs::read_dir(folder).is_ok_and(|entries| {
        entries.flatten().any(|e| {
            let path = e.path();
            let ext = path
                .extension()
                .and_then(|x| x.to_str())
                .unwrap_or_default();
            ext.eq_ignore_ascii_case("csproj") || ext.eq_ignore_ascii_case("sln")
        })
    });
    top_level_project
        || changed.iter().any(|path| {
            Path::new(path)
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|ext| SOURCE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        })
}

/// The folders where the user dismissed the git offer for good.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Offers {
    dismissed: BTreeSet<PathBuf>,
}

fn offers_path(changes_dir: &Path) -> PathBuf {
    changes_dir.join("offers.json")
}

fn read_offers(changes_dir: &Path) -> Offers {
    std::fs::read(offers_path(changes_dir))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// True when the user said "don't ask again" for `folder`.
#[must_use]
pub fn offer_dismissed(changes_dir: &Path, folder: &Path) -> bool {
    read_offers(changes_dir).dismissed.contains(folder)
}

/// Remembers "don't ask again" for `folder`.
///
/// # Errors
/// `Io` when the file cannot be written.
pub fn dismiss_offer(changes_dir: &Path, folder: &Path) -> Result<(), ChangesError> {
    let mut offers = read_offers(changes_dir);
    offers.dismissed.insert(folder.to_path_buf());
    let bytes = serde_json::to_vec_pretty(&offers)
        .map_err(|e| ChangesError::Parse(format!("offers.json: {e}")))?;
    let path = offers_path(changes_dir);
    let tmp = changes_dir.join("offers.json.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| ChangesError::io("write", &tmp, e))?;
    std::fs::rename(&tmp, &path).map_err(|e| ChangesError::io("replace", &path, e))
}

/// What `git init` + first commit would do in a folder (shown in the confirmation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct InitPlan {
    /// The folder.
    pub folder: String,
    /// Files the first commit would contain (counted up to 100 000).
    pub files: u32,
    /// Their total size in bytes.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub bytes: u64,
    /// Files left out for their size (relative paths).
    pub large: Vec<String>,
    /// Generated folders that would be committed (no `.gitignore` excludes them).
    pub generated: Vec<String>,
    /// The commit author when git has no `user.name`/`user.email` configured.
    pub fallback_author: Option<String>,
}

/// Author used when the user's git has no identity.
pub const FALLBACK_NAME: &str = "Vibe Work";
/// Author e-mail used when the user's git has no identity.
pub const FALLBACK_EMAIL: &str = "vibe-work@localhost";

/// Counts what a first commit of `folder` would contain.
///
/// # Errors
/// `Refused` when `folder` already has `.git`; `Io` when it cannot be read.
pub fn plan_init(folder: &Path) -> Result<InitPlan, ChangesError> {
    if folder.join(".git").exists() {
        return Err(ChangesError::Refused(
            "this folder already has version history".into(),
        ));
    }
    let gitignore = std::fs::read_to_string(folder.join(".gitignore")).unwrap_or_default();
    let generated: Vec<String> = GENERATED
        .iter()
        .filter(|name| {
            folder.join(name).is_dir()
                && !gitignore
                    .lines()
                    .any(|l| l.trim().trim_matches('/') == **name)
        })
        .map(|name| format!("{name}/"))
        .collect();
    let (mut files, mut bytes, mut large) = (0_u32, 0_u64, Vec::new());
    let mut stack = vec![folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| ChangesError::io("read", &dir, e))?;
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() && !entry.file_type().is_ok_and(|t| t.is_symlink()) {
                stack.push(entry.path());
            } else if meta.is_file() {
                if meta.len() > LARGE_UNTRACKED {
                    let rel = entry
                        .path()
                        .strip_prefix(folder)
                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default();
                    large.push(rel);
                } else {
                    files = files.saturating_add(1);
                    bytes = bytes.saturating_add(meta.len());
                }
            }
            if files >= 100_000 {
                stack.clear();
                break;
            }
        }
    }
    Ok(InitPlan {
        folder: folder.to_string_lossy().into_owned(),
        files,
        bytes,
        large,
        generated,
        fallback_author: (!has_identity(folder))
            .then(|| format!("{FALLBACK_NAME} <{FALLBACK_EMAIL}>")),
    })
}

fn has_identity(folder: &Path) -> bool {
    let get = |key: &str| {
        Call::new(folder, Index::User)
            .args(["config", "--get", key])
            .timeout(Duration::from_secs(10))
            .run()
            .is_ok_and(|v| !String::from_utf8_lossy(&v).trim().is_empty())
    };
    get("user.name") && get("user.email")
}

/// Runs `git init`, records every file (large ones left out) and makes the first commit. The user confirmed
/// exactly this (`plan_init` shows what it covers).
///
/// # Errors
/// `Refused` when the folder already has `.git`; git failures.
pub fn init_repository(folder: &Path) -> Result<InitPlan, ChangesError> {
    let plan = plan_init(folder)?;
    let user = Index::User;
    Call::new(folder, user).args(["init", "-q"]).run()?;
    let mut specs: Vec<u8> = b".\0".to_vec();
    for path in &plan.large {
        specs.extend_from_slice(
            crate::paths::exclude_literal(path)
                .to_string_lossy()
                .as_bytes(),
        );
        specs.push(0);
    }
    Call::new(folder, user)
        .args(["add", "-A", "--pathspec-from-file=-", "--pathspec-file-nul"])
        .stdin(&specs)
        .timeout(Duration::from_secs(300))
        .run()?;
    let mut commit = Call::new(folder, user);
    if plan.fallback_author.is_some() {
        commit = commit.args([
            "-c",
            &format!("user.name={FALLBACK_NAME}"),
            "-c",
            &format!("user.email={FALLBACK_EMAIL}"),
        ]);
    }
    commit
        .args([
            "commit",
            "-q",
            "--no-verify",
            "--allow-empty",
            "-m",
            "First commit (version history turned on in Vibe Work)",
        ])
        .timeout(Duration::from_secs(120))
        .run()?;
    Ok(plan)
}
