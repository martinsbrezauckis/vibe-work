//! The folder guard keeps one session's agent out of another session's copy: the agent of one session
//! never reads or changes another session's copy. Every copy lives under one `$VIBE_HOME/worktrees/<repo>-<hash>/<name>`
//! (Vibe 2.26.1 worktree layout), so a `find` that starts at the Vibe home reaches every other session's files.
//!
//! **One active session per copy**: a copy is held by the session working in it while that
//! session is active; another session is refused there while the holder is active, and may work there once the holder
//! is idle or closed (a later agent, a merge agent, a new session the user starts there). Who holds a copy and whether
//! it is active only the core knows: for every path inside ANY copy (the own one too) the hook asks it (`copy_check`,
//! `permits` below); no answer about another copy = refused.
//!
//! The rule, for a path P (resolved like the kernel: symlinks followed before each `..`, [`resolve`]), with V = the
//! Vibe home and W = V/worktrees: P inside a copy `W/<repo>/<name>` is decided by the core (above); V itself, W and a
//! `W/<repo>` folder (they hold several copies) are refused. Writes and unknown uses ([`Access`]: bash, any tool but `read_file`) are also refused anywhere else in V
//! (Vibe's `config.toml`, `hooks.toml`, the connector cache the model proxy relies on, and session data);
//! `read_file` may read there (Vibe's skills live under `logs/`). The user's project folders remain available.
//!
//! Choke points: the `pre_tool` hook (`hook.rs`) on EVERY tool call: any path-naming input field ([`PATH_FIELDS`],
//! absolute [`PATTERN_FIELDS`]) of any tool (Vibe's `read_file`, `write_file`, `edit`, legacy `grep`, `process.start`
//! `cwd`, an MCP server's tool, a future built-in: unknown tools fail closed), and the `command` of the shell tools
//! ([`SHELL_TOOLS`], [`check_command`]). Our own `mcp_vw.*` tools are skipped: they already work only inside the
//! session's folders (`session::inside` in the core).
//!
//! **What bash can and cannot be held to:** the guard reads the paths a command names: absolute paths, `~`, `$HOME`,
//! `$VIBE_HOME`, path-like executables and relative arguments (also after `cd`), every `cd`/`pushd` target and the
//! folder the shell is in for each later command, `--opt=<path>`, redirection targets and the folder part of a glob.
//! It cannot see paths a program computes itself (a script, `find / ...` from above V, `$(...)`, variables other than
//! those two, a symlink made in the same command): a guard against mistakes, not a sandbox (an OS sandbox is the
//! only full answer; the agent runs with the user's permissions).
//!
//! Depends on: `killparse_shell` (tokenizer), std. Called by: `hook`, tests.
//! Invariants: only reads the file system to resolve paths (no writes); an absent Vibe home has no copies to guard.
//! Failed resolution of a named path refuses its use.

use std::path::{Component, Path, PathBuf};

use crate::killparse_shell::{Segment, Word, segments};

/// The session's view of the copies: the Vibe home, its worktrees folder and the session's own copy (all resolved).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// `$VIBE_HOME`, resolved.
    pub vibe_home: PathBuf,
    /// `$VIBE_HOME/worktrees`, resolved.
    pub worktrees: PathBuf,
    /// The session's own copy (`worktrees/<repo>-<hash>/<name>`), when its `cwd` is inside one.
    pub own: Option<PathBuf>,
    /// The session's folder as Vibe reports it (`cwd`), for the refusal text.
    pub cwd: PathBuf,
}

/// `path` (absolute) resolved the way the kernel walks it: component by component, each existing prefix resolved
/// through the file system (symlinks followed) BEFORE a later `..` applies, so `own/link/../x` with `link` pointing
/// into another copy resolves next to the link's target, as the kernel would (folding `..`
/// lexically first gave `own/x`). A dangling symlink leaf is resolved through its target so creating that target
/// cannot escape the guard. Once an ordinary prefix does not exist the rest is folded lexically (the kernel would
/// fail there unless a separate operation first creates that prefix).
///
/// # Errors
/// A filesystem read fails for a reason other than a missing path, or a symlink chain exceeds 40 hops.
pub fn resolve(path: &Path) -> std::io::Result<PathBuf> {
    resolve_with_hops(path, &mut 0)
        .map(|real| normalize_prefix(&real))
        .map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!("could not resolve {} ({error})", path.display()),
            )
        })
}

// Keep the canonical Windows prefix even when no component exists yet (or the path is just a root).
// Using verbatim form preserves long paths and names that cannot safely be expressed in DOS form.
#[cfg(windows)]
fn normalize_prefix(path: &Path) -> PathBuf {
    let mut parts = path.components();
    let Some(Component::Prefix(prefix)) = parts.next() else {
        return path.to_path_buf();
    };
    let Some(root) = normalized_prefix(prefix.kind()) else {
        return path.to_path_buf();
    };
    let mut out = PathBuf::from(root);
    out.extend(parts);
    out
}

// Separate prefix spelling from the host parser so Windows prefix cases can also be checked on Linux.
#[cfg(any(windows, test))]
fn normalized_prefix(prefix: std::path::Prefix<'_>) -> Option<std::ffi::OsString> {
    use std::path::Prefix;

    match prefix {
        Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
            Some(format!(r"\\?\{}:", char::from(drive.to_ascii_uppercase())).into())
        }
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
            let mut root = std::ffi::OsString::from(r"\\?\UNC\");
            root.push(server.to_ascii_lowercase());
            root.push(r"\");
            root.push(share.to_ascii_lowercase());
            Some(root)
        }
        _ => None,
    }
}

#[cfg(not(windows))]
fn normalize_prefix(path: &Path) -> PathBuf {
    path.to_path_buf()
}

// Verbatim Windows components recognize only backslashes; Vibe can send forward slashes even with that prefix.
#[cfg(windows)]
fn normalize_separators(path: &Path) -> PathBuf {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    let units: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .map(|unit| {
            if unit == u16::from(b'/') {
                u16::from(b'\\')
            } else {
                unit
            }
        })
        .collect();
    PathBuf::from(std::ffi::OsString::from_wide(&units))
}

#[cfg(not(windows))]
fn normalize_separators(path: &Path) -> PathBuf {
    path.to_path_buf()
}

// canonicalize returns verbatim paths, but an ordinary input must keep Win32 filename semantics while probing
// later components (for example, `worktrees.` can name `worktrees`). Normalize identity only after walking it.
#[cfg(windows)]
fn path_for_probe(path: &Path, input: &Path) -> PathBuf {
    use std::path::Prefix;

    if matches!(input.components().next(), Some(Component::Prefix(prefix)) if prefix.kind().is_verbatim())
    {
        return path.to_path_buf();
    }
    let mut parts = path.components();
    let Some(Component::Prefix(prefix)) = parts.next() else {
        return path.to_path_buf();
    };
    let root = match prefix.kind() {
        Prefix::VerbatimDisk(drive) => std::ffi::OsString::from(format!("{}:", char::from(drive))),
        Prefix::VerbatimUNC(server, share) => {
            let mut root = std::ffi::OsString::from(r"\\");
            root.push(server);
            root.push(r"\");
            root.push(share);
            root
        }
        _ => return path.to_path_buf(),
    };
    let mut out = PathBuf::from(root);
    out.extend(parts);
    out
}

#[cfg(not(windows))]
fn path_for_probe(path: &Path, _input: &Path) -> PathBuf {
    path.to_path_buf()
}

// PathBuf::join on a verbatim base folds `..` before a junction can be followed. Keep the raw spelling until resolve.
fn join_for_resolution(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute()
        || matches!(path.components().next(), Some(Component::Prefix(_)))
        || base.as_os_str().is_empty()
    {
        return path.to_path_buf();
    }
    let mut out = if path.has_root() {
        match base.components().next() {
            Some(Component::Prefix(prefix)) => prefix.as_os_str().to_os_string(),
            _ => std::ffi::OsString::new(),
        }
    } else {
        let mut out = base.as_os_str().to_os_string();
        out.push(std::path::MAIN_SEPARATOR_STR);
        out
    };
    out.push(path.as_os_str());
    PathBuf::from(out)
}

// A junction is a reparse point even on Windows versions that do not classify it as a symbolic link.
// If canonicalization fails, read_link must resolve its target or fail closed rather than treating it as absent.
fn is_link(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        meta.file_type().is_symlink() || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

fn resolve_with_hops(path: &Path, hops: &mut usize) -> std::io::Result<PathBuf> {
    use std::io::ErrorKind::{InvalidFilename, NotFound};

    let path = normalize_separators(path);
    let mut out = PathBuf::new();
    let mut exists = true;
    for part in path.components() {
        // Verbatim components retain literal dot names; the guard still walks them in kernel order.
        let part = match part {
            Component::Normal(name) if name == "." => Component::CurDir,
            Component::Normal(name) if name == ".." => Component::ParentDir,
            part => part,
        };
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
                exists = path_for_probe(&out, &path).try_exists()?;
            }
            Component::Normal(name) => {
                out.push(name);
                if exists {
                    let probe = path_for_probe(&out, &path);
                    match std::fs::canonicalize(&probe) {
                        Ok(real) => out = real,
                        Err(error) => match std::fs::symlink_metadata(&probe) {
                            Ok(meta) if is_link(&meta) => {
                                *hops += 1;
                                if *hops > 40 {
                                    return Err(std::io::Error::other("too many symbolic links"));
                                }
                                let target = std::fs::read_link(&probe)?;
                                let target = if target.is_absolute() {
                                    target
                                } else {
                                    join_for_resolution(
                                        out.parent().unwrap_or(Path::new("")),
                                        &target,
                                    )
                                };
                                out = resolve_with_hops(&target, hops)?;
                                exists = path_for_probe(&out, &path).try_exists()?;
                            }
                            Ok(_) if matches!(error.kind(), NotFound | InvalidFilename) => {
                                exists = false;
                            }
                            Err(meta_error)
                                if matches!(meta_error.kind(), NotFound | InvalidFilename)
                                    && matches!(error.kind(), NotFound | InvalidFilename) =>
                            {
                                exists = false;
                            }
                            Err(meta_error) => return Err(meta_error),
                            _ => return Err(error),
                        },
                    }
                }
            }
            Component::Prefix(_) | Component::RootDir => {
                // A drive prefix alone (`C:`) is not the absolute path being resolved. It can fail or
                // refer to the drive's current directory. Only probe after the first normal component.
                out.push(part.as_os_str());
            }
        }
    }
    Ok(out)
}

/// What a tool does with a path, for [`Layout::check`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Reads only (`read_file`): the Vibe home is readable (Vibe's skills live under `logs/`), other copies are not.
    Read,
    /// Writes (`write_file`, `edit`): nothing in the Vibe home outside the own copy (its config, hooks, connector
    /// cache, sessions are Vibe Work's control files.
    Write,
    /// Unknown (bash, `process.start`, any other tool): judged as a write (fails closed).
    Unknown,
    /// The shell's working directory, rather than a path the command targets.
    Context,
}

// Reserve spellings Windows can interpret as this file before it exists, including default data streams.
fn approval_path(path: &Path, data: &Path) -> bool {
    path.strip_prefix(data)
        .ok()
        .and_then(|relative| relative.components().next())
        .and_then(|part| part.as_os_str().to_str())
        .is_some_and(|name| {
            name.split(':')
                .next()
                .unwrap_or_default()
                .trim_end_matches(['.', ' '])
                .eq_ignore_ascii_case("plugin-approvals.json")
        })
}

impl Layout {
    /// The layout for a session whose folder is `cwd`, with the Vibe home `vibe_home`.
    ///
    /// # Errors
    /// The Vibe home is not absolute, or a folder cannot be resolved. The error names the failing path.
    pub fn new(vibe_home: &Path, cwd: &Path) -> std::io::Result<Self> {
        if vibe_home.as_os_str().is_empty() || !vibe_home.is_absolute() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "Vibe home is not an absolute folder: {}",
                    vibe_home.display()
                ),
            ));
        }
        let named_worktrees = normalize_prefix(&normalize_separators(vibe_home)).join("worktrees");
        let named_cwd = normalize_prefix(&normalize_separators(cwd));
        let vibe_home = resolve(vibe_home)?;
        let worktrees = resolve(&vibe_home.join("worktrees"))?;
        let here = resolve(cwd)?;
        // A cwd naming a managed copy must still resolve into that same copy.
        let relative = named_cwd
            .strip_prefix(&named_worktrees)
            .or_else(|_| named_cwd.strip_prefix(&worktrees));
        let mut expected = None;
        if let Ok(relative) = relative {
            let parts: Vec<_> = relative.components().take(2).collect();
            if let [Component::Normal(repo), Component::Normal(copy)] = parts.as_slice() {
                expected = Some(resolve(&worktrees.join(repo).join(copy))?);
            }
        } else {
            // Resolve prefixes in order too: aliases and case variations can hide the named home.
            for prefix in cwd.ancestors().collect::<Vec<_>>().into_iter().rev() {
                let real = resolve(prefix)?;
                if let Ok(relative) = real.strip_prefix(&worktrees) {
                    let parts: Vec<_> = relative.components().take(2).collect();
                    if let [Component::Normal(repo), Component::Normal(copy)] = parts.as_slice() {
                        expected = Some(resolve(&worktrees.join(repo).join(copy))?);
                        break;
                    }
                }
            }
        }
        if let Some(expected) = expected
            && (!expected.starts_with(&worktrees) || !here.starts_with(&expected))
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "session folder {} resolves outside its copy {}",
                    cwd.display(),
                    expected.display()
                ),
            ));
        }
        let own = here.strip_prefix(&worktrees).ok().and_then(|inner| {
            let mut parts = inner.components();
            match (parts.next(), parts.next()) {
                (Some(repo), Some(name)) => Some(worktrees.join(repo).join(name)),
                _ => None,
            }
        });
        Ok(Self {
            vibe_home,
            worktrees,
            own,
            cwd: cwd.to_path_buf(),
        })
    }

    /// The copy folder (`worktrees/<repo>/<name>`) a resolved path lies in, if any.
    fn copy_of(&self, real: &Path) -> Option<PathBuf> {
        let inner = real.strip_prefix(&self.worktrees).ok()?;
        let mut parts = inner.components();
        match (parts.next(), parts.next()) {
            (Some(repo), Some(name)) => Some(self.worktrees.join(repo).join(name)),
            _ => None,
        }
    }

    /// The refusal for `path` (absolute or relative to `base`) used as `access`, or `None` when it may be used.
    /// `permits(copy, own)`: the core's answer whether this session may work in `copy` now (one active session per
    /// copy), `own` = the copy holding the session's folder; asked for every path inside a copy.
    #[must_use]
    pub fn check(
        &self,
        path: &Path,
        base: &Path,
        access: Access,
        permits: &Permits<'_>,
    ) -> Option<String> {
        let joined = join_for_resolution(base, path);
        let own = match &self.own {
            Some(own) => format!("Your own folder is {}.", own.display()),
            None => format!("Your folder is {}.", self.cwd.display()),
        };
        let real = match resolve(&joined) {
            Ok(real) => real,
            Err(error) => {
                return Some(format!(
                    "Not run: {} could not be resolved for the folder guard ({error}). {own}",
                    joined.display()
                ));
            }
        };
        if access != Access::Read
            && let Some(data) = self.vibe_home.parent()
            && (approval_path(&real, data) || access != Access::Context && data.starts_with(&real))
        {
            return Some(format!(
                "Not run: {} contains Vibe Work's plugin approvals; change plugins in Tools. {own}",
                joined.display()
            ));
        }
        if let Some(copy) = self.copy_of(&real) {
            let is_own = self.own.as_ref() == Some(&copy);
            return match permits(&copy, is_own) {
                Ok(()) => None,
                Err(why) => Some(format!(
                    "Not run: {} is in a session copy you cannot use now: {why}. {own}",
                    joined.display()
                )),
            };
        }
        if real == self.vibe_home
            || real == self.worktrees
            || real.parent() == Some(self.worktrees.as_path())
        {
            return Some(format!(
                "Not run: {} holds the sessions' copies (Vibe Work gives each session its own copy). {own} Search and \
                 work only inside it.",
                joined.display()
            ));
        }
        if access != Access::Read && real.starts_with(&self.vibe_home) {
            return Some(format!(
                "Not run: {} is Vibe Work's own configuration and session data (the Vibe home); the agent does not \
                 change it. {own} Read skills with the skill tool or read_file.",
                joined.display()
            ));
        }
        None
    }
}

/// The core's answer for one copy: `(copy, is_own)` -> `Ok` = may work there now, `Err(why)`.
pub type Permits<'a> = dyn Fn(&Path, bool) -> Result<(), String> + 'a;

/// Tool input fields that name a path, checked for EVERY tool (Vibe 2.26.1: `read_file`/`write_file` `path` (the hook
/// sees `file_path`), `edit` `file_path`, legacy `grep` `path`, `process.start` `cwd`, the scratchpad `path`; any
/// other tool, an MCP server's or a future built-in, with one of these names is checked too: fails closed).
pub const PATH_FIELDS: &[&str] = &[
    "file_path",
    "path",
    "paths",
    "filePath",
    "cwd",
    "dir",
    "directory",
    "folder",
    "root",
    "target",
    "target_path",
    "destination",
    "source",
];

/// Fields that hold a glob or search pattern: the part before the first glob character is judged as a folder (relative
/// to the tool's own `path`/`cwd`, else the session's folder); a relative one with a `..` component is refused.
pub const PATTERN_FIELDS: &[&str] = &["pattern", "glob", "include"];

/// The shell-like tools whose `command` is read as a shell command line ([`check_command`]).
pub const SHELL_TOOLS: &[&str] = &[
    "bash",
    "process_start",
    "git_bash",
    "windows_shell",
    "start",
];

fn strings(value: &serde_json::Value) -> Vec<&str> {
    match value {
        serde_json::Value::String(text) => vec![text.as_str()],
        serde_json::Value::Array(items) => {
            items.iter().filter_map(serde_json::Value::as_str).collect()
        }
        _ => Vec::new(),
    }
}

/// Every path a tool's input names ([`PATH_FIELDS`], absolute [`PATTERN_FIELDS`]), checked. `None` = may run.
#[must_use]
pub fn check_tool_input(
    layout: &Layout,
    tool_input: &serde_json::Value,
    access: Access,
    permits: &Permits<'_>,
) -> Option<String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    for key in PATH_FIELDS {
        for text in tool_input.get(*key).map(strings).unwrap_or_default() {
            if text.trim().is_empty() {
                continue;
            }
            let text = expand(text, home.as_deref(), &layout.vibe_home);
            let folder = folder_part(&text);
            let folder = if folder.is_empty() { "." } else { folder };
            if let Some(refusal) = layout.check(Path::new(folder), &layout.cwd, access, permits) {
                return Some(refusal);
            }
        }
    }
    // Patterns: the part before the first glob character is a folder, relative to the tool's own
    // `path`/`cwd` when it names one, else to the session's folder; it is judged like any path. A relative pattern with
    // a `..` component is refused outright (it climbs out of the folder the tool searches).
    let base = ["path", "cwd", "dir", "directory"]
        .iter()
        .find_map(|key| tool_input.get(*key).and_then(serde_json::Value::as_str))
        .map(|b| expand(b, home.as_deref(), &layout.vibe_home))
        .map_or_else(
            || layout.cwd.clone(),
            |b| join_for_resolution(&layout.cwd, Path::new(&b)),
        );
    for key in PATTERN_FIELDS {
        for text in tool_input.get(*key).map(strings).unwrap_or_default() {
            let text = expand(text, home.as_deref(), &layout.vibe_home);
            let folder = folder_part(&text);
            let relative = !Path::new(&text).is_absolute();
            if relative
                && Path::new(folder)
                    .components()
                    .any(|c| c == Component::ParentDir)
            {
                return Some(format!(
                    "Not run: the pattern {text} climbs out of the folder it searches (`..`); search inside your own \
                     folder {}.",
                    layout.cwd.display()
                ));
            }
            let folder = if folder.is_empty() { "." } else { folder };
            if let Some(refusal) = layout.check(Path::new(folder), &base, access, permits) {
                return Some(refusal);
            }
        }
    }
    None
}

/// `word` with a leading `~`, `$HOME`, `${HOME}`, `$VIBE_HOME` or `${VIBE_HOME}` replaced.
fn expand(word: &str, home: Option<&Path>, vibe_home: &Path) -> String {
    let vibe = vibe_home.to_string_lossy();
    let mut out = word.to_owned();
    for name in ["${VIBE_HOME}", "$VIBE_HOME"] {
        out = out.replace(name, &vibe);
    }
    if let Some(home) = home {
        let home = home.to_string_lossy();
        for name in ["${HOME}", "$HOME"] {
            out = out.replace(name, &home);
        }
        if out == "~" || out.starts_with("~/") {
            out = format!("{home}{}", &out[1..]);
        }
    }
    out
}

/// The folder part of a word that may name a path: the text after `=` of an option, cut at the first glob
/// character; `None` when the word does not look like a path.
fn path_part(word: &str) -> Option<&str> {
    let text = match word.split_once('=') {
        Some((key, value)) if key.starts_with('-') => value,
        _ => word,
    };
    let text = folder_part(text);
    let looks = Path::new(text).is_absolute()
        || text.starts_with('/')
        || text == "."
        || text == ".."
        || text.starts_with("./")
        || text.starts_with("../")
        || (text.contains(std::path::MAIN_SEPARATOR) && !text.contains("://"))
        || (text.contains('/') && !text.contains("://"));
    looks.then_some(text)
}

// The `?` in the Windows verbatim prefix is part of the root, not a wildcard.
fn folder_part(text: &str) -> &str {
    let start = if text.starts_with(r"\\?\") { 4 } else { 0 };
    let end = text[start..]
        .find(['*', '?', '[', '{'])
        .map_or(text.len(), |index| start + index);
    &text[..end]
}

/// A shell word that names a path, including an existing bare filename (which may be a symlink).
fn shell_path_part<'a>(word: &'a str, dir: &Path) -> Option<&'a str> {
    path_part(word).or_else(|| {
        (!word.is_empty()
            && std::fs::symlink_metadata(join_for_resolution(dir, Path::new(word))).is_ok())
        .then_some(word)
    })
}

// The tokenizer keeps command substitutions in one word. After the two known home variables are expanded,
// remaining shell syntax means the word is computed at runtime, not a filename to resolve literally.
fn computed_shell_word(source: &Word, expanded: &str) -> bool {
    source.expanded && (expanded.contains('$') || expanded.contains('`'))
}

mod mentions;
use mentions::raw_mentions;

/// The refusal for a bash `command` run in `cwd`, or `None`. `home` = `$HOME` of the hook's process.
#[must_use]
pub fn check_command(
    layout: &Layout,
    command: &str,
    home: Option<&Path>,
    permits: &Permits<'_>,
) -> Option<String> {
    if let Some(refusal) = layout.check(&layout.cwd, &layout.cwd, Access::Context, permits) {
        return Some(refusal);
    }
    let expanded = expand(command, home, &layout.vibe_home);
    // The raw text first: absolute paths under the Vibe home (both its given and resolved spellings).
    let given = std::env::var_os("VIBE_HOME").map(PathBuf::from);
    let mut roots: Vec<String> = vec![layout.vibe_home.to_string_lossy().into_owned()];
    if let Some(given) = given.filter(|g| g.is_absolute()) {
        roots.push(given.to_string_lossy().into_owned());
    }
    for root in &roots {
        for mention in raw_mentions(&expanded, root) {
            let folder = folder_part(&mention);
            if let Some(refusal) =
                layout.check(Path::new(folder), &layout.cwd, Access::Unknown, permits)
            {
                return Some(refusal);
            }
        }
    }
    let refusal = check_segments(layout, segments(command), home, permits);
    #[cfg(windows)]
    {
        // The hook supplies no shell identity. Check both Bash escaping and Windows' literal path separators.
        // Each interpretation starts at the original cwd, so a cd in one cannot affect the other.
        refusal.or_else(|| {
            check_segments(
                layout,
                crate::killparse_shell::windows_segments(command),
                home,
                permits,
            )
        })
    }
    #[cfg(not(windows))]
    refusal
}

fn check_segments(
    layout: &Layout,
    parsed: Vec<Segment>,
    home: Option<&Path>,
    permits: &Permits<'_>,
) -> Option<String> {
    // Then word by word, following `cd` so relative paths resolve where the shell would be. Every `cd`/`pushd` target
    // is checked whatever its shape (`cd sibling`), and each later segment checks the folder the shell is in by then
    // (`cd x && ls` runs `ls` inside x.
    let mut dir = layout.cwd.clone();
    let mut moved = false;
    for segment in parsed {
        let words: Vec<String> = segment
            .words
            .iter()
            .map(|w| expand(&w.text, home, &layout.vibe_home))
            .collect();
        let first_command = segment
            .words
            .iter()
            .take_while(|word| {
                word.text.split_once('=').is_some_and(|(name, _)| {
                    !name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
                })
            })
            .count();
        if moved && let Some(refusal) = layout.check(&dir, &dir, Access::Context, permits) {
            return Some(refusal);
        }
        let is_cd = words
            .get(first_command)
            .is_some_and(|w| w == "cd" || w == "pushd");
        for (index, word) in words.iter().enumerate() {
            // A leading shell assignment is not a path named `NAME=...`.
            if index < first_command {
                continue;
            }
            if computed_shell_word(&segment.words[index], word) {
                continue;
            }
            // Bare executable names use PATH, not the current folder, even if a local file has that name.
            if index == first_command
                && !word.contains('/')
                && !word.contains(std::path::MAIN_SEPARATOR)
                && !word.starts_with('.')
                && !word.starts_with('~')
            {
                continue;
            }
            let Some(part) =
                shell_path_part(word, &dir).or((index == first_command).then_some(word.as_str()))
            else {
                continue;
            };
            let part = if part.is_empty() { "." } else { part };
            if let Some(refusal) = layout.check(
                Path::new(part),
                &dir,
                if is_cd {
                    Access::Context
                } else {
                    Access::Unknown
                },
                permits,
            ) {
                return Some(refusal);
            }
        }
        for redirect in &segment.redirects {
            let target = expand(&redirect.text, home, &layout.vibe_home);
            if computed_shell_word(redirect, &target) {
                continue;
            }
            if let Some(part) = shell_path_part(&target, &dir)
                && let Some(refusal) = layout.check(
                    Path::new(part),
                    &dir,
                    if is_cd {
                        Access::Context
                    } else {
                        Access::Unknown
                    },
                    permits,
                )
            {
                return Some(refusal);
            }
        }
        if is_cd
            && let Some(target) = words.get(first_command + 1).filter(|w| !w.starts_with('-'))
            && !computed_shell_word(&segment.words[first_command + 1], target)
        {
            let target = Path::new(target);
            if let Some(refusal) = layout.check(target, &dir, Access::Context, permits) {
                return Some(refusal);
            }
            dir = join_for_resolution(&dir, target);
            moved = true;
        }
    }
    None
}

#[cfg(test)]
mod tests;

#[cfg(all(test, windows))]
mod windows_tests;
