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
//! `read_file` may read there (including skills and their references). Outside V, app data is private except for
//! the temporary chat's own scratch folder.
//! The user's project folders remain available.
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

pub mod diagnostics;
mod resolution;
#[cfg(test)]
use resolution::normalized_prefix;
pub use resolution::resolve;
use resolution::{join_for_resolution, normalize_prefix, normalize_separators};

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

/// What a tool does with a path, for [`Layout::check`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Reads only (`read_file`): the Vibe home is readable; copies still require permission.
    Read,
    /// Writes (`write_file`, `edit`): nothing in the Vibe home outside the own copy (its config, hooks, connector
    /// cache, sessions are Vibe Work's control files.
    Write,
    /// Unknown (bash, `process.start`, any other tool): judged as a write (fails closed).
    Unknown,
    /// The shell's working directory, rather than a path the command targets.
    Context,
}

/// The Chrome Control state folder beside the Vibe home (the core's `chrome_control` module keeps the user's
/// permissions and the helper's copy there).
pub const CHROME_CONTROL_STATE: &str = "chrome-control";

// Reserve spellings Windows can interpret as this file before it exists, including default data streams.
fn approval_path(path: &Path, data: &Path) -> bool {
    data_entry_named(path, data, "plugin-approvals.json")
}

/// `path` is `data/<name>` or inside it, in any spelling Windows reads as that name (case, trailing dots and
/// spaces, a default data stream).
fn data_entry_named(path: &Path, data: &Path, wanted: &str) -> bool {
    path.strip_prefix(data)
        .ok()
        .and_then(|relative| relative.components().next())
        .and_then(|part| part.as_os_str().to_str())
        .is_some_and(|name| {
            name.split(':')
                .next()
                .unwrap_or_default()
                .trim_end_matches(['.', ' '])
                .eq_ignore_ascii_case(wanted)
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
            return Err(diagnostics::outside_copy(cwd, expected));
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
            Some(own) => format!("Your own folder is {}.", shown::shown(own)),
            None => format!("Your folder is {}.", shown::shown(&self.cwd)),
        };
        let real = match resolve(&joined) {
            Ok(real) => real,
            Err(error) => {
                eprintln!("folder path {}: {error}", joined.display());
                return Some(format!(
                    "Not run: Vibe Work could not resolve {}. Check the path and its links, then try again. {own}",
                    shown::shown(&joined)
                ));
            }
        };
        if let Some(data) = self.vibe_home.parent()
            && data_entry_named(&real, data, CHROME_CONTROL_STATE)
        {
            return Some(format!(
                concat!(
                    "Not run: Vibe Work's browser setup and permissions ({}); only the user changes them, in ",
                    vw_vocabulary::settings_path!("Browser", "{}"),
                    ". {}"
                ),
                shown::shown(&joined),
                vw_vocabulary::label("workNames", "yourChrome"),
                own
            ));
        }
        if access != Access::Read
            && let Some(data) = self.vibe_home.parent()
            && approval_path(&real, data)
        {
            return Some(format!(
                "Not run: Vibe Work's plugin approvals ({}); Ask the user to change plugins in Tools. {own}",
                shown::shown(data)
            ));
        }
        if access != Access::Read
            && let Some(data) = self.vibe_home.parent()
            && data_entry_named(&real, data, "sessions")
        {
            return Some(format!(
                "Not run: Vibe Work's own session data ({}); the agent does not change it. {own}",
                shown::shown(&joined)
            ));
        }
        if let Some(copy) = self.copy_of(&real) {
            let is_own = self.own.as_ref() == Some(&copy);
            return match app_data::copy_permit(self, &copy, is_own, permits) {
                Ok(()) => None,
                Err(why) => Some(format!(
                    "Not run: a session copy you cannot use now ({}): {}. {own}",
                    shown::shown(&joined),
                    why.trim_end_matches('.')
                )),
            };
        }
        if real == self.vibe_home
            || real == self.worktrees
            || real.parent() == Some(self.worktrees.as_path())
        {
            return Some(format!(
                "Not run: the sessions' copies ({}); Vibe Work gives each session its own copy. {own} Search and \
                 work only inside it.",
                shown::shown(&joined)
            ));
        }
        if access != Access::Read && real.starts_with(&self.vibe_home) {
            return Some(format!(
                "Not run: Vibe Work's own configuration and session data, the Vibe home ({}); the agent does not \
                 change it. {own} Read skills with the skill tool or read_file. To change files there, propose the change with propose_protected_change; the user reviews it.",
                shown::shown(&joined)
            ));
        }
        app_data::check(self, &real, access, &own)
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

mod app_data;
mod mentions;
mod shown;
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
    for root in &mentions::with_windows_spellings(roots) {
        for mention in raw_mentions(&expanded, root) {
            let folder = mentions::native(folder_part(&mention));
            if let Some(refusal) =
                layout.check(Path::new(&*folder), &layout.cwd, Access::Unknown, permits)
            {
                return Some(refusal);
            }
        }
    }
    let refusal = check_segments(layout, &segments(command), home, permits);
    #[cfg(windows)]
    {
        // The hook supplies no shell identity. Check both Bash escaping and Windows' literal path separators.
        // Each interpretation starts at the original cwd, so a cd in one cannot affect the other.
        refusal.or_else(|| {
            check_segments(
                layout,
                &crate::killparse_shell::windows_segments(command),
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
    parsed: &[Segment],
    home: Option<&Path>,
    permits: &Permits<'_>,
) -> Option<String> {
    // Then word by word, following `cd` so relative paths resolve where the shell would be. Every `cd`/`pushd` target
    // is checked whatever its shape (`cd sibling`), and each later segment checks the folder the shell is in by then
    // (`cd x && ls` runs `ls` inside x.
    let mut dir = layout.cwd.clone();
    let mut moved = false;
    for (segment, data) in parsed.iter().zip(crate::heredoc_input::inputs(parsed)) {
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
        let access = if is_cd {
            Access::Context
        } else {
            Access::Unknown
        };
        for (index, word) in words.iter().enumerate() {
            // A leading shell assignment is not a path named `NAME=...`.
            if index < first_command
                || computed_shell_word(&segment.words[index], word)
                || data && crate::heredoc_input::operator(&segment.words, index)
            {
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
            let part = mentions::native(if part.is_empty() { "." } else { part });
            if let Some(refusal) = layout.check(Path::new(&*part), &dir, access, permits) {
                return Some(refusal);
            }
        }
        for redirect in &segment.redirects {
            let target = expand(&redirect.text, home, &layout.vibe_home);
            if computed_shell_word(redirect, &target) {
                continue;
            }
            if let Some(part) = shell_path_part(&target, &dir)
                && let Some(refusal) =
                    layout.check(Path::new(&*mentions::native(part)), &dir, access, permits)
            {
                return Some(refusal);
            }
        }
        if is_cd
            && let Some(target) = words.get(first_command + 1).filter(|w| !w.starts_with('-'))
            && !computed_shell_word(&segment.words[first_command + 1], target)
        {
            // The folder the shell moves to: a Git Bash `/c/...` target is that Windows folder, also for the
            // relative paths checked after it.
            let target = mentions::native(target);
            let target = Path::new(&*target);
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
