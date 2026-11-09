//! Turning the session tools on for a run: find the `vw-agent-tools` binary, give the core its
//! config, add the guards to Vibe Work's own `$VIBE_HOME/hooks.toml` (one combined pre-tool entry),
//! and load the built-in plugin.
//!
//! - **Binary:** next to the app executable (a bundled sidecar), else in the cargo target folder of a
//!   development build. Missing: the tools are off, the hook block is removed (a `strict` hook whose binary is
//!   gone would deny every bash call), and the reason is returned for the log.
//! - **Git Bash (Windows):** a copy of the binary runs Vibe's Git Bash in a kill-on-close job ([`crate::git_bash`]).
//! - **`hooks.toml`:** Vibe Work owns this `VIBE_HOME`; our hook lives in a marked block that is replaced
//!   on every start (new binary or data path) and anything outside it is kept. Vibe 3.0.4 can run native plugin
//!   hooks; VW refuses those subprocesses because they bypass our tool guards. Our guards stay in this file,
//!   also in auto-approve, and the bundled plugin contains no hooks.
//! - **Plugin:** `vibe-app-server --additional-builtin-plugin-root <dir>` with `<dir>/vw/` (skill
//!   `vw:processes`). Built-in plugins run fully trusted: the folder comes from the app's own files only.
//! - **The Vibe Work note** (`vw_core_api::working_note`): the short "where you are, which skill when" map
//!   at the top of our `$VIBE_HOME/AGENTS.md`, written when the plugin loads and removed when it does not (it names
//!   the plugin's skills). A failure to write it is reported in the log line; the tools stay on.
//!
//! Depends on: vw-core-api (`AgentToolsConfig`), vw-supervisor (`SupervisorConfig`). Called by: `app`
//! (startup), the private test hosts.
//! Invariant: nothing is written outside `VIBE_HOME/hooks.toml`, our block in `VIBE_HOME/AGENTS.md` and the given data
//! folder.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use vw_agent_tools::hook::Guard;
use vw_core_api::{AgentToolsConfig, CoreConfig};
use vw_supervisor::SupervisorConfig;

/// File name of the binary on this OS.
pub const BINARY: &str = if cfg!(windows) {
    "vw-agent-tools.exe"
} else {
    "vw-agent-tools"
};

const BEGIN: &str =
    "# >>> vibe-work guards (managed by Vibe Work: this block is rewritten at every start) >>>";
const END: &str = "# <<< vibe-work guards <<<";
/// The markers of the legacy single-hook block: removed like ours, so an upgrade never leaves the
/// old `vibe-work-kill-guard` entry next to the new ones.
const LEGACY_BEGIN: &str =
    "# >>> vibe-work kill guard (managed by Vibe Work: this block is rewritten at every start) >>>";
const LEGACY_END: &str = "# <<< vibe-work kill guard <<<";

/// The binary next to `exe` (the running app), if present.
#[must_use]
pub fn binary_next_to(exe: &Path) -> Option<PathBuf> {
    let candidate = exe.parent()?.join(BINARY);
    candidate.is_file().then_some(candidate)
}

/// Quotes a path for the Unix hook's shell line.
fn shell_quote(path: &Path) -> String {
    let text = path.to_string_lossy();
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Command-name segments join literally in both shells (PowerShell argument mode differs).
fn both_quote(text: &str) -> String {
    let mut quoted = String::from("'");
    for c in text.chars() {
        // PowerShell's single-quote delimiters include all four typographic apostrophes.
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}') {
            quoted.push_str("'\"");
            quoted.push(c);
            quoted.push_str("\"'");
        } else {
            quoted.push(c);
        }
    }
    quoted.push('\'');
    quoted
}

/// An unquoted root makes PowerShell parse a command rather than a string expression.
/// All user-controlled text is quoted; the absolute path bypasses PATH and current-folder lookup.
fn windows_hook_binary(binary: &Path) -> String {
    let plain = vw_platform::plain_path(binary);
    let text = plain.to_string_lossy().replace('\\', "/");
    let root = if text.starts_with("//") {
        2
    } else if text.starts_with('/') {
        1
    } else if text.as_bytes().get(1..3) == Some(b":/") && text.as_bytes()[0].is_ascii_alphabetic() {
        3
    } else {
        0
    };
    let (root, rest) = text.split_at(root);
    format!("{root}{}", both_quote(rest))
}

#[cfg(any(windows, test))]
fn hook_environment(
    endpoint: &Path,
    existing_exclusions: Option<std::ffi::OsString>,
) -> Vec<(String, std::ffi::OsString)> {
    let mut exclusions = existing_exclusions.unwrap_or_default();
    if exclusions != "*" {
        if !exclusions.is_empty() {
            exclusions.push(";");
        }
        exclusions.push(format!("{}=", vw_agent_tools::hook::ENV_ENDPOINT_FILE));
    }
    vec![
        (
            vw_agent_tools::hook::ENV_ENDPOINT_FILE.to_owned(),
            endpoint.as_os_str().to_owned(),
        ),
        ("MSYS2_ENV_CONV_EXCL".to_owned(), exclusions),
    ]
}

/// Installs the hook block and the child environment as one launcher configuration.
/// Real-server fixtures use this same path so Windows hooks can find their endpoint.
///
/// # Errors
/// The Vibe home's managed hook file could not be written.
pub fn configure_hooks(
    supervisor: &mut SupervisorConfig,
    config: &AgentToolsConfig,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        let exclusions = supervisor
            .extra_env
            .iter()
            .rev()
            .find(|(name, _)| name.eq_ignore_ascii_case("MSYS2_ENV_CONV_EXCL"))
            .map(|(_, value)| value.clone())
            .or_else(|| std::env::var_os("MSYS2_ENV_CONV_EXCL"));
        supervisor
            .extra_env
            .extend(hook_environment(&config.endpoint_file(), exclusions));
    }
    write_hooks(
        &supervisor.vibe_home,
        Some(&hook_block(&config.binary, &config.endpoint_file())),
    )
}

/// Windows invokes the binary directly with a line parsed identically by Git Bash and PowerShell.
/// Unix keeps its existing shell quoting.
fn hook_command(binary: &Path, endpoint: &Path, action: &str, windows: bool) -> String {
    if windows {
        format!("{} {action}", windows_hook_binary(binary))
    } else {
        format!(
            "{} {action} --endpoint-file {}",
            shell_quote(binary),
            shell_quote(endpoint)
        )
    }
}

/// A TOML basic string.
fn toml_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Our managed hooks: one strict pre-tool entry for the union of the original matchers (`*`),
/// followed by the unchanged post-tool allowed note. The combined CLI preserves per-guard selection and order.
/// Existing markers let both newer and older hosts replace the block on every start.
#[must_use]
pub fn hook_block(binary: &Path, endpoint_file: &Path) -> String {
    let mut out = format!("{BEGIN}\n");
    for guard in std::iter::once(None).chain(Guard::AFTER.map(Some)) {
        let (name, event, matcher, timeout, description, action) = match guard {
            None => (
                // Interrupted sessions keep their old bindings. Retain the first wildcard guard's ID
                // so their original pre-tool action still resolves to the complete combined check.
                Guard::Safety.hook_name(),
                "pre_tool",
                "*",
                f64::INFINITY,
                "Vibe Work: safety reminders, processes, folders, Git, connectors and tool permissions",
                "kill-guard".to_owned(),
            ),
            Some(guard) => (
                guard.hook_name(),
                guard.event(),
                guard.matcher(),
                guard.timeout_secs(),
                "Vibe Work: a command the user allowed in a guard's dialog says so in its result",
                format!("guard {}", guard.arg()),
            ),
        };
        let command = hook_command(binary, endpoint_file, &action, cfg!(windows));
        let strict = event == "pre_tool";
        let _ = write!(
            out,
            "[[hooks]]\nname = {}\ntype = {}\nmatch = {}\nstrict = {strict}\ntimeout = {timeout}\ndescription = {}\ncommand = {}\n\n",
            toml_string(name),
            toml_string(event),
            toml_string(matcher),
            toml_string(description),
            toml_string(&command)
        );
    }
    out.truncate(out.trim_end().len());
    out.push('\n');
    out.push_str(END);
    out.push('\n');
    out
}

/// `existing` with our block replaced by `block` (or removed when `None`); other content kept.
#[must_use]
pub fn with_block(existing: &str, block: Option<&str>) -> String {
    let mut kept = String::new();
    let mut inside = false;
    for line in existing.lines() {
        if line.trim() == BEGIN || line.trim() == LEGACY_BEGIN {
            inside = true;
            continue;
        }
        if inside {
            if line.trim() == END || line.trim() == LEGACY_END {
                inside = false;
            }
            continue;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    let mut out = kept.trim_end().to_owned();
    if let Some(block) = block {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(block);
    } else if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Writes (or removes) our block in `vibe_home/hooks.toml`.
///
/// # Errors
/// The I/O error text when the file cannot be read or written.
pub fn write_hooks(vibe_home: &Path, block: Option<&str>) -> Result<(), String> {
    let path = vibe_home.join("hooks.toml");
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let updated = with_block(&existing, block);
    if updated == existing || (updated.is_empty() && existing.is_empty()) {
        return Ok(());
    }
    std::fs::create_dir_all(vibe_home).map_err(|e| format!("{}: {e}", vibe_home.display()))?;
    std::fs::write(&path, updated).map_err(|e| format!("{}: {e}", path.display()))
}

/// Turns the session tools on for this run: core config, `hooks.toml`, plugin root. With `binary = None` the
/// tools are off and our hook block is removed. Returns a line for the log.
///
/// # Errors
/// A text reason when `hooks.toml` cannot be updated (the tools stay off then: a stale strict hook could deny
/// every bash call).
pub fn prepare(
    supervisor: &mut SupervisorConfig,
    core: &mut CoreConfig,
    data_dir: &Path,
    binary: Option<PathBuf>,
    plugin_root: Option<&Path>,
) -> Result<String, String> {
    for path in [
        Some(supervisor.vibe_home.as_path()),
        Some(data_dir),
        binary.as_deref(),
        plugin_root,
    ]
    .into_iter()
    .flatten()
    {
        if !path.is_absolute() {
            return Err(format!(
                "agent tool path must be absolute: {}",
                path.display()
            ));
        }
    }
    // Windows: Vibe's Git Bash in a kill-on-close job (the copy is of this binary), or our shell line removed.
    #[cfg(windows)]
    let shell = format!(
        "; {}",
        crate::git_bash::prepare(supervisor, data_dir, binary.as_deref())
    );
    #[cfg(not(windows))]
    let shell = String::new();
    let Some(binary) = binary else {
        write_hooks(&supervisor.vibe_home, None)?;
        let note = note_line(&supervisor.vibe_home, false);
        return Ok(format!("agent tools off: {BINARY} not found{note}{shell}"));
    };
    let config = AgentToolsConfig::new(
        binary.clone(),
        data_dir.join("agent-tools"),
        supervisor.home.as_deref(),
    );
    configure_hooks(supervisor, &config)?;
    core.agent_tools = Some(config);
    let plugin = plugin_root.filter(|r| r.join("vw").join("plugin.json").is_file());
    let staged = match (plugin, vw_core_api::working_note::vault_capture_line()) {
        (Some(root), Some(line)) => Some(crate::vault_plugin::stage(root, data_dir, line)?),
        _ => None,
    };
    let plugin = staged.as_deref().or(plugin);
    if let Some(root) = plugin {
        supervisor
            .extra_args
            .push("--additional-builtin-plugin-root".into());
        supervisor.extra_args.push(root.as_os_str().to_owned());
    }
    let note = note_line(&supervisor.vibe_home, plugin.is_some());
    Ok(format!("agent tools on: {}{note}{shell}", binary.display()))
}

/// Writes or removes the Vibe Work note in `vibe_home/AGENTS.md`; the text to add to the log line (empty when all
/// went well).
fn note_line(vibe_home: &Path, on: bool) -> String {
    match vw_core_api::working_note::write(vibe_home, on) {
        Ok(_) => String::new(),
        Err(reason) => format!("; Vibe Work note not updated: {reason}"),
    }
}

#[cfg(test)]
#[path = "agent_tools_tests.rs"]
mod tests;
