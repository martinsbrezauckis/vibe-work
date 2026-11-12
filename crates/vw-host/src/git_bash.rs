//! Windows: Vibe's Git Bash runs inside a kill-on-close job, so Stop and the bash timeout end every program a command
//! started (`vw_agent_tools::git_bash`).
//!
//! At each start, when the session tools are on and Vibe can find Git Bash:
//! - the tools binary is copied to `<data>/shell/bash.exe` (Vibe accepts a configured Git Bash only by that name; the
//!   copy is refreshed when it differs, written beside and renamed over);
//! - Vibe Work's own `VIBE_HOME/config.toml` gets `[tools.git_bash] shell = "<that copy>"`;
//! - the Vibe server's environment gets [`vw_agent_tools::git_bash::ENV_REAL`] = the real Git Bash, and
//!   [`ARG_CONV_EXCL`] so `taskkill /F /PID n` reaches Windows as written ([`arg_conv_excl`]).
//!
//! A `shell` the user set to something else is kept (that shell then runs without the job, as before). Without Git
//! Bash, without the tools binary or when the copy fails, our `shell` line is removed again: Vibe then finds its own
//! shell as before, and a stale line never points Vibe at a wrapper with no real Git Bash behind it.
//!
//! Without Git Bash Vibe runs the agent's commands in PowerShell; [`prepare`] then also runs the PowerShell step
//! (`crate::powershell_shell`), which uses the same config editing and copy for `[tools.powershell]`.
//!
//! Depends on: `toml_edit` (the file keeps its comments and layout), `vw_platform::replace_private`. Called by:
//! `agent_tools::prepare` (Windows), tests (the pure parts on every OS).
//! Invariants: only Vibe Work's own `VIBE_HOME`; a file that is not valid TOML is left byte-identical.

use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Table, value};

/// The copy's folder under the data folder.
pub const DIR: &str = "shell";

/// What the config should say about our wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wanted<'a> {
    /// `[tools.git_bash] shell` is our wrapper at this path.
    Ours(&'a str),
    /// No wrapper this run: our line (the wrapper at this path) goes, anything else stays.
    None(&'a str),
}

/// What [`with_shell`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shell {
    /// The text changed; the new text.
    Changed(String),
    /// Already as wanted.
    Same,
    /// The user set another shell: kept.
    UsersOwn(String),
    /// Not valid TOML, or `tools` / `tools.git_bash` is not a table: left as it is.
    Skipped,
}

/// The config text with `[tools.git_bash] shell` as `wanted` asks. Pure.
#[must_use]
pub fn with_shell(existing: &str, wanted: &Wanted<'_>) -> Shell {
    with_tool_shell(existing, TOOL, wanted)
}

/// Vibe's command tool for Git Bash (its `[tools.<name>]` table).
pub const TOOL: &str = "git_bash";

/// The config text with `[tools.<tool>] shell` as `wanted` asks. Pure.
#[must_use]
pub fn with_tool_shell(existing: &str, tool: &str, wanted: &Wanted<'_>) -> Shell {
    let Ok(mut config) = existing.parse::<DocumentMut>() else {
        return Shell::Skipped;
    };
    let ours = match wanted {
        Wanted::Ours(path) | Wanted::None(path) => *path,
    };
    let current = config
        .get("tools")
        .and_then(|tools| tools.get(tool))
        .and_then(|bash| bash.get("shell"))
        .map(|shell| shell.as_str().map(str::to_owned));
    match (wanted, current) {
        (_, Some(Some(shell))) if shell != ours => Shell::UsersOwn(shell),
        (_, Some(None)) => Shell::Skipped,
        (Wanted::Ours(_), Some(Some(_))) | (Wanted::None(_), None) => Shell::Same,
        (Wanted::None(_), Some(Some(_))) => {
            let Some(bash) = config
                .get_mut("tools")
                .and_then(|tools| tools.get_mut(tool))
                .and_then(Item::as_table_like_mut)
            else {
                return Shell::Skipped;
            };
            bash.remove("shell");
            Shell::Changed(config.to_string())
        }
        (Wanted::Ours(path), None) => {
            let tools = config
                .entry("tools")
                .or_insert_with(|| {
                    let mut table = Table::new();
                    table.set_implicit(true);
                    Item::Table(table)
                })
                .as_table_like_mut();
            let Some(tools) = tools else {
                return Shell::Skipped;
            };
            let Some(bash) = tools
                .entry(tool)
                .or_insert(Item::Table(Table::new()))
                .as_table_like_mut()
            else {
                return Shell::Skipped;
            };
            bash.insert("shell", value(*path));
            Shell::Changed(config.to_string())
        }
    }
}

/// Git Bash as Vibe finds it ([`vw_agent_tools::shell_find::git_bash`], which our `bash.exe` copy also uses when a
/// Vibe CLI outside the app runs it). Pure.
#[must_use]
pub fn find_real(
    var: impl Fn(&str) -> Option<String>,
    is_file: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    vw_agent_tools::shell_find::git_bash(var, is_file)
}

/// Copies `tools` to `<data>/shell/bash.exe` unless the copy there is already the same file; returns the copy.
///
/// # Errors
/// The folder, the copy or the rename failed (an old copy in use by a still-running command keeps the rename from
/// replacing it).
pub fn place_copy(tools: &Path, data_dir: &Path) -> std::io::Result<PathBuf> {
    place_copy_as(tools, data_dir, vw_agent_tools::git_bash::FILE_NAME)
}

/// Copies `tools` to `<data>/shell/<file_name>` unless the copy there is already the same file; returns the copy.
///
/// # Errors
/// As [`place_copy`].
pub fn place_copy_as(tools: &Path, data_dir: &Path, file_name: &str) -> std::io::Result<PathBuf> {
    let dir = data_dir.join(DIR);
    std::fs::create_dir_all(&dir)?;
    let copy = dir.join(file_name);
    if std::fs::read(&copy).ok() == Some(std::fs::read(tools)?) {
        return Ok(copy);
    }
    let fresh = copy.with_extension("new");
    std::fs::copy(tools, &fresh)?;
    std::fs::rename(&fresh, &copy)?;
    Ok(copy)
}

/// Brings Vibe Work's `vibe_home/config.toml` in line with `wanted`; a line for the log.
#[must_use]
pub fn write_config(vibe_home: &Path, wanted: &Wanted<'_>) -> String {
    write_tool_config(vibe_home, TOOL, wanted)
}

/// [`write_config`] for `[tools.<tool>] shell`.
#[must_use]
pub fn write_tool_config(vibe_home: &Path, tool: &str, wanted: &Wanted<'_>) -> String {
    let path = vibe_home.join("config.toml");
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return format!("config.toml not read: {error}"),
    };
    match with_tool_shell(&existing, tool, wanted) {
        Shell::Changed(updated) => match vw_platform::replace_private(&path, updated.as_bytes()) {
            Ok(()) => "config updated".to_owned(),
            Err(error) => format!("config.toml not written: {error}"),
        },
        Shell::Same => "config already set".to_owned(),
        Shell::UsersOwn(shell) => {
            format!(
                "the config names its own {tool} shell ({shell}), kept: Stop may leave its programs running"
            )
        }
        Shell::Skipped => {
            "config.toml not changed (not valid TOML or an unexpected tools table)".to_owned()
        }
    }
}

/// The Windows start-up step (see the module text); a line for the log.
#[cfg(windows)]
#[must_use]
pub fn prepare(
    supervisor: &mut vw_supervisor::SupervisorConfig,
    data_dir: &Path,
    tools: Option<&Path>,
) -> String {
    let ours = data_dir.join(DIR).join(vw_agent_tools::git_bash::FILE_NAME);
    let ours_text = ours.to_string_lossy().into_owned();
    let real = find_real(|name| std::env::var(name).ok(), Path::is_file);
    // Without Git Bash, Vibe runs the agent's commands in PowerShell (Vibe 3.1.1 `vibe/app_server/_runtime.py`
    // `_command_environment_mode`).
    let powershell = crate::powershell_shell::prepare(supervisor, data_dir, tools, real.is_none());
    let bash = prepare_bash(supervisor, data_dir, tools, real, &ours_text);
    format!("{bash}; {powershell}")
}

/// The Git Bash half of [`prepare`]; a line for the log.
#[cfg(windows)]
fn prepare_bash(
    supervisor: &mut vw_supervisor::SupervisorConfig,
    data_dir: &Path,
    tools: Option<&Path>,
    real: Option<PathBuf>,
    ours_text: &str,
) -> String {
    let outcome = match (tools, real) {
        (Some(tools), Some(real)) => place_copy(tools, data_dir)
            .map(|copy| (copy, real))
            .map_err(|error| error.to_string()),
        (None, _) => Err("the tools binary is missing".to_owned()),
        (_, None) => Err("Vibe finds no Git Bash".to_owned()),
    };
    match outcome {
        Ok((copy, real)) => {
            let wanted = copy.to_string_lossy().into_owned();
            let config = write_config(&supervisor.vibe_home, &Wanted::Ours(&wanted));
            supervisor.extra_env.push((
                vw_agent_tools::git_bash::ENV_REAL.to_owned(),
                real.clone().into_os_string(),
            ));
            let excl = arg_conv_excl(std::env::var(ARG_CONV_EXCL).ok().as_deref());
            supervisor
                .extra_env
                .push((ARG_CONV_EXCL.to_owned(), excl.into()));
            format!(
                "Git Bash in a job: {} runs {} ({config})",
                copy.display(),
                real.display()
            )
        }
        Err(why) => {
            let config = write_config(&supervisor.vibe_home, &Wanted::None(ours_text));
            format!("Git Bash without a job: {why} ({config})")
        }
    }
}

/// Git Bash variable: arguments starting with one of these `;`-separated prefixes reach Windows programs unconverted.
pub const ARG_CONV_EXCL: &str = "MSYS2_ARG_CONV_EXCL";

/// `taskkill`'s flags as an agent writes them: Git Bash otherwise turns `/F` into `F:/` and `/PID` into
/// `C:/Program Files/Git/PID`, so a Windows kill the user allowed then fails. Lower-case `/f` and `/t` are not listed:
/// they would also match the drive path `/f/...` and `/tmp/...`.
const TASKKILL_FLAGS: &str = "/F;/T;/PID;/IM;/pid;/im";

/// The `MSYS2_ARG_CONV_EXCL` value for the agent's Git Bash: the user's own list (if any) plus `taskkill`'s flags;
/// a user's `*` (convert nothing) stays as it is.
#[must_use]
pub fn arg_conv_excl(existing: Option<&str>) -> String {
    match existing.map(str::trim) {
        None | Some("") => TASKKILL_FLAGS.to_owned(),
        Some("*") => "*".to_owned(),
        Some(own) => format!("{own};{TASKKILL_FLAGS}"),
    }
}
