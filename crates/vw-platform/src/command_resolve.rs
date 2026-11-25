//! Program resolution for command-started MCP servers: the name the person typed becomes the absolute path that
//! is shown in the confirmation and saved in Vibe's config, decided per operating system.
//!
//! Depends on: std only. Called by: vw-core-api (`command_servers`, at add and edit time).
//! Invariants:
//! - **The program is never run here.** Resolution only looks at the file system. The macOS login-shell probe runs
//!   the person's own shell to print its `PATH`, never the program being resolved.
//! - Paths are resolved lexically (`.` and `..` removed); links are kept, so an environment's `python` link is not
//!   replaced by the interpreter it points to.
//! - Windows: names resolve through `PATHEXT` (`npx` -> `npx.cmd`) as Vibe's own launcher does
//!   (`mcp/os/win32/utilities.py`: `shutil.which`, then `.cmd .bat .exe .ps1`). Vibe then starts the file with
//!   `CreateProcess`, which runs `.cmd` and `.bat` through `cmd.exe` without escaping the arguments, so a script is
//!   accepted only when its path and every argument have no character `cmd.exe` could read as syntax
//!   ([`check_launch`]); other file types are not programs.
//! - macOS: an app started from the Dock does not inherit the shell's `PATH`, so the login shell's `PATH` is asked
//!   (`-l`, and `-i -l` only when the program is still not found), with a short timeout and no input.
//! - The fake [`Probe`] used by the tests stands in for the host, so every system's rules run on any machine.

use std::path::Path;

/// The operating system whose rules apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsKind {
    /// `PATHEXT`, `\` and drive letters.
    Windows,
    /// The login shell's `PATH` is merged in.
    MacOs,
    /// `PATH` only.
    Linux,
}

impl OsKind {
    /// The system this build runs on.
    #[must_use]
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Linux
        }
    }
}

/// What a path is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// A file (links followed).
    File,
    /// A folder.
    Dir,
}

/// What resolution may ask the machine.
pub trait Probe {
    /// An environment variable.
    fn var(&self, name: &str) -> Option<String>;
    /// What `path` is, or `None` when it does not exist.
    fn kind(&self, path: &str) -> Option<EntryKind>;
    /// True when the file may be run (the execute bit on Unix; always true on Windows).
    fn is_executable(&self, path: &str) -> bool;
    /// The `PATH` of the person's login shell (`interactive`: also read the interactive start files); macOS only.
    fn login_path(&self, interactive: bool) -> Option<String>;
}

/// Why a program was not resolved. `Display` is a plain sentence with the way to fix it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    /// Nothing typed.
    #[error("Type the program to run: a name such as npx or uvx, or its full path.")]
    Empty,
    /// A control character or a character that reorders text.
    #[error(
        "The program name has a control or direction-changing character: type it again as plain text."
    )]
    BadCharacters,
    /// Not on the search path and not a file.
    #[error(
        "Command not found: {name}. Check the spelling, install the program, or pick its full path."
    )]
    NotFound {
        /// What was typed.
        name: String,
    },
    /// A relative path with no working folder to start from.
    #[error(
        "A path that starts from the current folder needs a working folder. Choose one, or give the program's full path."
    )]
    RelativeNeedsFolder,
    /// A folder, not a file.
    #[error("{path} is a folder, not a program. Pick the program file inside it.")]
    IsFolder {
        /// The resolved path.
        path: String,
    },
    /// A file the account may not run.
    #[error(
        "{path} does not have permission to run. Give it run permission (chmod +x) or pick another program."
    )]
    NotRunnable {
        /// The resolved path.
        path: String,
    },
    /// A Windows file that Vibe cannot start (not .exe, .com, .cmd or .bat).
    #[error(
        "{path} is not a program Vibe can start. Pick a .exe, or start your script through its interpreter (for example python.exe with the script as an argument)."
    )]
    NotAProgram {
        /// The resolved path.
        path: String,
    },
    /// A Windows script with a character `cmd.exe` would read as syntax.
    #[error(
        "{path} is a Windows script, and Windows runs scripts through cmd.exe, which reads characters such as & | < > ^ % \" and spaces in arguments as commands. Use plain arguments, or start the tool's .exe directly (for example node.exe with the script as an argument)."
    )]
    ScriptArguments {
        /// The resolved path.
        path: String,
    },
}

fn is_bidi(c: char) -> bool {
    matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

fn sep(os: OsKind) -> char {
    if os == OsKind::Windows { '\\' } else { '/' }
}

fn is_absolute(os: OsKind, text: &str) -> bool {
    if os == OsKind::Windows {
        let b = text.as_bytes();
        (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/'))
            || text.starts_with("\\\\")
    } else {
        text.starts_with('/')
    }
}

/// `text` with one separator, no `.` or empty parts and `..` applied (lexically; links are not followed).
fn normalise(os: OsKind, text: &str) -> String {
    let sep = sep(os);
    let unified: String = if os == OsKind::Windows {
        text.replace('/', "\\")
    } else {
        text.to_owned()
    };
    let (root, rest) = if os == OsKind::Windows {
        if let Some(rest) = unified.strip_prefix("\\\\") {
            ("\\\\".to_owned(), rest.to_owned())
        } else {
            let (drive, rest) = unified.split_at(2.min(unified.len()));
            (
                format!("{drive}\\"),
                rest.trim_start_matches('\\').to_owned(),
            )
        }
    } else {
        ("/".to_owned(), unified.trim_start_matches('/').to_owned())
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split(sep) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    format!("{root}{}", parts.join(&sep.to_string()))
}

fn join(os: OsKind, dir: &str, name: &str) -> String {
    let sep = sep(os);
    if dir.ends_with(sep) {
        format!("{dir}{name}")
    } else {
        format!("{dir}{sep}{name}")
    }
}

fn pathext(probe: &dyn Probe) -> Vec<String> {
    let raw = probe
        .var("PATHEXT")
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_owned());
    raw.split(';')
        .map(|e| e.trim().to_ascii_lowercase())
        .filter(|e| e.starts_with('.') && e.len() > 1)
        .collect()
}

fn extension(os: OsKind, path: &str) -> Option<String> {
    let leaf = path
        .rsplit(|c| c == '/' || (os == OsKind::Windows && c == '\\'))
        .next()?;
    let (stem, ext) = leaf.rsplit_once('.')?;
    (!stem.is_empty()).then(|| format!(".{}", ext.to_ascii_lowercase()))
}

/// The file names to try for `name` in one folder.
fn names(os: OsKind, name: &str, probe: &dyn Probe) -> Vec<String> {
    if os != OsKind::Windows {
        return vec![name.to_owned()];
    }
    let exts = pathext(probe);
    let mut list = Vec::new();
    if extension(os, name).is_some_and(|e| exts.contains(&e)) {
        list.push(name.to_owned());
    } else {
        list.extend(exts.iter().map(|e| format!("{name}{e}")));
    }
    list
}

/// The first runnable file among `candidates`; else what explains the miss.
fn first_runnable(
    os: OsKind,
    candidates: &[String],
    probe: &dyn Probe,
) -> Result<String, Option<ResolveError>> {
    let mut why = None;
    for path in candidates {
        match probe.kind(path) {
            Some(EntryKind::File) if os == OsKind::Windows || probe.is_executable(path) => {
                return Ok(path.clone());
            }
            Some(EntryKind::File) => {
                why.get_or_insert(ResolveError::NotRunnable { path: path.clone() });
            }
            Some(EntryKind::Dir) => {
                why.get_or_insert(ResolveError::IsFolder { path: path.clone() });
            }
            None => {}
        }
    }
    Err(why)
}

fn search_dirs(os: OsKind, probe: &dyn Probe, extra: Option<&str>) -> Vec<String> {
    let split = if os == OsKind::Windows { ';' } else { ':' };
    let mut dirs: Vec<String> = Vec::new();
    for source in [extra.map(str::to_owned), probe.var("PATH")]
        .into_iter()
        .flatten()
    {
        for dir in source.split(split) {
            let dir = dir.trim().trim_matches('"');
            if !dir.is_empty() && is_absolute(os, dir) && !dirs.iter().any(|d| d == dir) {
                dirs.push(dir.to_owned());
            }
        }
    }
    dirs
}

/// Resolves `command` (as typed) to the absolute program path for `os`.
///
/// # Errors
/// [`ResolveError`]: nothing typed, odd characters, not found, a folder, not runnable, or a relative path
/// without a working folder.
pub fn resolve(
    os: OsKind,
    command: &str,
    cwd: Option<&str>,
    probe: &dyn Probe,
) -> Result<String, ResolveError> {
    let command = command.trim();
    if command.is_empty() {
        return Err(ResolveError::Empty);
    }
    if command.chars().any(|c| c.is_control() || is_bidi(c)) {
        return Err(ResolveError::BadCharacters);
    }
    let not_found = || ResolveError::NotFound {
        name: command.chars().take(80).collect(),
    };
    let home_path = if os == OsKind::Windows {
        None
    } else {
        command.strip_prefix("~/").and_then(|rest| {
            probe
                .var("HOME")
                .map(|home| join(os, home.trim_end_matches('/'), rest))
        })
    };
    let explicit = home_path.is_some()
        || command.contains('/')
        || (os == OsKind::Windows && command.contains('\\'));
    if explicit {
        let full = match home_path {
            Some(path) => path,
            None if is_absolute(os, command) => command.to_owned(),
            None => join(
                os,
                cwd.filter(|c| is_absolute(os, c))
                    .ok_or(ResolveError::RelativeNeedsFolder)?,
                command,
            ),
        };
        let full = normalise(os, &full);
        let tries: Vec<String> = names(os, &full, probe);
        return first_runnable(os, &tries, probe).map_err(|why| why.unwrap_or_else(not_found));
    }
    let mut miss = None;
    // Rounds: the process PATH, then on macOS the login shell's, then the interactive shell's.
    let rounds: &[Option<bool>] = if os == OsKind::MacOs {
        &[None, Some(false), Some(true)]
    } else {
        &[None]
    };
    for round in rounds {
        let extra = match round {
            None => None,
            Some(interactive) => match probe.login_path(*interactive) {
                Some(path) => Some(path),
                None => continue,
            },
        };
        let mut tries = Vec::new();
        for dir in search_dirs(os, probe, extra.as_deref()) {
            tries.extend(
                names(os, command, probe)
                    .into_iter()
                    .map(|n| join(os, &dir, &n)),
            );
        }
        match first_runnable(os, &tries, probe) {
            Ok(path) => return Ok(path),
            Err(why) => miss = miss.or(why),
        }
    }
    Err(miss.unwrap_or_else(not_found))
}

/// Characters allowed in a script's arguments: nothing `cmd.exe` reads as syntax and no space (a space makes Python
/// add quotes, and `cmd.exe` then strips the wrong ones).
fn plain_argument(arg: &str) -> bool {
    !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.:\\/@=,+~".contains(c))
}

/// Characters allowed in a script's own path: spaces are fine (the path is quoted), `&<>()@^|` and `%` are not.
fn plain_script_path(path: &str) -> bool {
    path.chars()
        .all(|c| c.is_ascii_alphanumeric() || " _-.:\\/+~,=".contains(c) || !c.is_ascii())
}

/// Whether Vibe can start `program` with `args` on `os` (the Windows script rule in the module docs).
///
/// # Errors
/// [`ResolveError::NotAProgram`] or [`ResolveError::ScriptArguments`] on Windows; nothing on other systems.
pub fn check_launch(os: OsKind, program: &str, args: &[String]) -> Result<(), ResolveError> {
    if os != OsKind::Windows {
        return Ok(());
    }
    match extension(os, program).as_deref() {
        Some(".exe" | ".com") => Ok(()),
        Some(".cmd" | ".bat") => {
            if plain_script_path(program) && args.iter().all(|a| plain_argument(a)) {
                Ok(())
            } else {
                Err(ResolveError::ScriptArguments {
                    path: program.to_owned(),
                })
            }
        }
        _ => Err(ResolveError::NotAProgram {
            path: program.to_owned(),
        }),
    }
}

/// The machine this build runs on.
pub struct HostProbe;

impl Probe for HostProbe {
    fn var(&self, name: &str) -> Option<String> {
        std::env::var_os(name).and_then(|v| v.into_string().ok())
    }

    fn kind(&self, path: &str) -> Option<EntryKind> {
        let meta = std::fs::metadata(Path::new(path)).ok()?;
        Some(if meta.is_dir() {
            EntryKind::Dir
        } else {
            EntryKind::File
        })
    }

    #[cfg(unix)]
    fn is_executable(&self, path: &str) -> bool {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }

    #[cfg(not(unix))]
    fn is_executable(&self, _path: &str) -> bool {
        true
    }

    fn login_path(&self, interactive: bool) -> Option<String> {
        if !cfg!(target_os = "macos") {
            return None;
        }
        login_shell_path(interactive)
    }
}

/// Resolves on this machine.
///
/// # Errors
/// See [`resolve`].
pub fn resolve_here(command: &str, cwd: Option<&str>) -> Result<String, ResolveError> {
    resolve(OsKind::current(), command, cwd, &HostProbe)
}

/// The login shell's `PATH` (`$SHELL`, else `/bin/zsh`), printed between markers so start-file chatter is ignored.
/// No input, a 4 second limit, output cut at 64 KiB.
fn login_shell_path(interactive: bool) -> Option<String> {
    use std::io::Read;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    const MARK: &str = "__VW_PATH__";
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| s.starts_with('/') && Path::new(s).is_file())
        .unwrap_or_else(|| "/bin/zsh".to_owned());
    let mut command = crate::process_command::hidden(shell);
    command.arg(if interactive { "-ilc" } else { "-lc" });
    command.arg(format!("printf '%s%s%s' {MARK} \"$PATH\" {MARK}"));
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().ok()?;
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut text = Vec::new();
        let _ = out.by_ref().take(64 * 1024).read_to_end(&mut text);
        text
    });
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    let text = String::from_utf8_lossy(&reader.join().ok()?).into_owned();
    let (_, rest) = text.split_once(MARK)?;
    let (path, _) = rest.split_once(MARK)?;
    (!path.is_empty()).then(|| path.to_owned())
}

#[cfg(test)]
#[path = "command_resolve_tests.rs"]
mod tests;
