//! Turning the session tools on for a run: find the `vw-agent-tools` binary, give the core its
//! config, add the guards to Vibe Work's own `$VIBE_HOME/hooks.toml` (one named `[[hooks]]` entry per guard),
//! and load the built-in plugin.
//!
//! - **Binary:** next to the app executable (a bundled sidecar), else in the cargo target folder of a
//!   development build. Missing: the tools are off, the hook block is removed (a `strict` hook whose binary is
//!   gone would deny every bash call), and the reason is returned for the log.
//! - **Git Bash (Windows):** a copy of the binary runs Vibe's Git Bash in a kill-on-close job ([`crate::git_bash`]).
//! - **`hooks.toml`:** Vibe Work owns this `VIBE_HOME`; our hook lives in a marked block that is replaced
//!   on every start (new binary or data path) and anything outside it is kept. Hooks in a built-in plugin never
//!   fire, so this file is the only place that works, also in auto-approve.
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

/// A fixed PowerShell launcher is parseable in both Git Bash and PowerShell.
/// Paths stay inside its UTF-16 script, so neither outer shell expands path characters.
fn hook_command(binary: &Path, endpoint: &Path, guard: Guard, windows: bool) -> String {
    if windows {
        use base64::Engine as _;
        let quote = |path: &Path| format!("'{}'", path.to_string_lossy().replace('\'', "''"));
        let script = format!(
            "& {} guard {} --endpoint-file {}; exit $LASTEXITCODE",
            quote(binary),
            guard.arg(),
            quote(endpoint)
        );
        let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        format!("powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand {encoded}")
    } else {
        format!(
            "{} guard {} --endpoint-file {}",
            shell_quote(binary),
            guard.arg(),
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

/// What each guard tells a reader of `hooks.toml` (Vibe shows the hook's name, not this).
fn guard_description(guard: Guard) -> &'static str {
    match guard {
        Guard::Safety => {
            "Vibe Work: reminders before dangerous commands (deleting the home folder, disk tools, shutdown, the user's own rules)"
        }
        Guard::Process => {
            "Vibe Work: literal kill commands aimed at other sessions' or apps' processes, or with nonliteral targets, wait for the user"
        }
        Guard::Git => {
            "Vibe Work: literal git push and GitHub writes are the user's Push / Create PR buttons; rewrites outside the own copy branch wait for the user"
        }
        Guard::Folder => {
            "Vibe Work: another session's copy is not this session's to read or change"
        }
        Guard::Connector => {
            "Vibe Work: connector tools that change a repository on a code forge are the user's Push / Create PR buttons"
        }
        Guard::Receipt => {
            "Vibe Work: wait for permission and bind the decision to its originating agent and call"
        }
    }
}

/// Our `hooks.toml` block for `binary` and the core's endpoint file: one `[[hooks]]` entry per guard,
/// in [`Guard::ALL`] order, each named so Vibe's refusal notice says which guard spoke.
/// The folder guard precedes git so another session's copy is refused before git can ask for approval.
#[must_use]
pub fn hook_block(binary: &Path, endpoint_file: &Path) -> String {
    let mut out = format!("{BEGIN}\n");
    for guard in Guard::ALL {
        let command = hook_command(binary, endpoint_file, guard, cfg!(windows));
        let _ = write!(
            out,
            "[[hooks]]\nname = {}\ntype = \"pre_tool\"\nmatch = {}\nstrict = true\ntimeout = {}\ndescription = {}\ncommand = {}\n\n",
            toml_string(guard.hook_name()),
            toml_string(guard.matcher()),
            guard.timeout_secs(),
            toml_string(guard_description(guard)),
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
    write_hooks(
        &supervisor.vibe_home,
        Some(&hook_block(&binary, &config.endpoint_file())),
    )?;
    core.agent_tools = Some(config);
    let plugin = plugin_root.filter(|r| r.join("vw").join("plugin.json").is_file());
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
mod tests {
    use super::{Guard, Path, hook_command};
    use base64::Engine as _;

    #[test]
    fn windows_hooks_keep_paths_out_of_both_outer_shells() {
        let line = hook_command(
            Path::new(r"C:\a b\$cash`tick'quote\vw-agent-tools.exe"),
            Path::new(r"C:\a b\hook.json"),
            Guard::Safety,
            true,
        );
        let encoded = line.split_whitespace().last().unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let script = String::from_utf16(&units).unwrap();
        assert_eq!(
            script,
            r"& 'C:\a b\$cash`tick''quote\vw-agent-tools.exe' guard safety --endpoint-file 'C:\a b\hook.json'; exit $LASTEXITCODE"
        );
        assert!(
            line.bytes()
                .all(|c| c.is_ascii_alphanumeric() || b" -+/=.".contains(&c))
        );
    }
}
