//! Windows: when Vibe runs the agent's commands in PowerShell (it finds no Git Bash), the PowerShell it starts is
//! Vibe Work's copy of the tools binary (`vw_agent_tools::powershell`). The copy runs the real PowerShell in a
//! kill-on-close job and without the server's `GIT_REDIRECT_STDIN=NUL` (`vw_supervisor` `env.rs`, the Vibe 3.1.1 git
//! start-up hang), so input a command pipes into git reaches git. Vibe starts PowerShell with `-NoProfile`, so a
//! profile cannot do this.
//!
//! At each start, when Vibe will use PowerShell and the tools binary and a PowerShell are there:
//! - the tools binary is copied to `<data>/shell/<pwsh.exe | powershell.exe>`, the real program's own name (Vibe
//!   accepts a configured PowerShell only by these names, `_processes/_windows.py` `_shell_family`);
//! - Vibe Work's own `VIBE_HOME/config.toml` gets `[tools.powershell] shell = "<that copy>"`;
//! - the Vibe server's environment gets [`vw_agent_tools::powershell::ENV_REAL`] = the real PowerShell.
//!
//! Otherwise (Git Bash found, no tools binary, no PowerShell, a failed copy) our `shell` line is removed again, and a
//! `shell` the user set to something else is kept, as for Git Bash (`crate::git_bash`).
//!
//! Depends on: `crate::git_bash` (config editing and the copy). Called by: `git_bash::prepare` (Windows), tests (the
//! pure parts on every OS).

use std::path::{Path, PathBuf};

/// Vibe's command tool for PowerShell (its `[tools.<name>]` table).
pub const TOOL: &str = "powershell";

/// PowerShell as Vibe finds it when no shell is configured ([`vw_agent_tools::shell_find::powershell`], which our
/// PowerShell copy also uses when a Vibe CLI outside the app runs it). Pure.
#[must_use]
pub fn find_real(
    var: impl Fn(&str) -> Option<String>,
    is_file: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    vw_agent_tools::shell_find::powershell(var, is_file)
}

/// Our copy for the real PowerShell `real`: `<data>/shell/<the real program's file name>`. Pure.
#[must_use]
pub fn copy_path(data_dir: &Path, real: &Path) -> Option<PathBuf> {
    let name = real.file_name()?.to_str()?.to_ascii_lowercase();
    vw_agent_tools::powershell::FILE_NAMES
        .contains(&name.as_str())
        .then(|| data_dir.join(crate::git_bash::DIR).join(name))
}

/// Which of our possible copies `existing` names in `[tools.powershell] shell` (for removal), else the first name's.
/// Pure.
#[must_use]
pub fn ours_in(existing: &str, data_dir: &Path) -> String {
    let ours: Vec<String> = vw_agent_tools::powershell::FILE_NAMES
        .iter()
        .map(|name| {
            data_dir
                .join(crate::git_bash::DIR)
                .join(name)
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let current = existing
        .parse::<toml_edit::DocumentMut>()
        .ok()
        .and_then(|config| {
            config
                .get("tools")
                .and_then(|tools| tools.get(TOOL))
                .and_then(|table| table.get("shell"))
                .and_then(|shell| shell.as_str().map(str::to_owned))
        });
    current
        .filter(|shell| ours.contains(shell))
        .unwrap_or_else(|| ours[0].clone())
}

/// The Windows start-up step (see the module text); `used` is whether Vibe will run commands in PowerShell. A line for
/// the log.
#[cfg(windows)]
#[must_use]
pub fn prepare(
    supervisor: &mut vw_supervisor::SupervisorConfig,
    data_dir: &Path,
    tools: Option<&Path>,
    used: bool,
) -> String {
    use crate::git_bash::{Wanted, place_copy_as, write_tool_config};
    let real = find_real(|name| std::env::var(name).ok(), Path::is_file);
    let outcome = match (used, tools, real) {
        (false, _, _) => Err("Vibe uses Git Bash".to_owned()),
        (true, None, _) => Err("the tools binary is missing".to_owned()),
        (true, _, None) => Err("Vibe finds no PowerShell".to_owned()),
        (true, Some(tools), Some(real)) => copy_path(data_dir, &real)
            .ok_or_else(|| format!("unexpected PowerShell name {}", real.display()))
            .and_then(|copy| {
                let name = copy
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_owned();
                place_copy_as(tools, data_dir, &name).map_err(|error| error.to_string())
            })
            .map(|copy| (copy, real)),
    };
    match outcome {
        Ok((copy, real)) => {
            let wanted = copy.to_string_lossy().into_owned();
            let config = write_tool_config(&supervisor.vibe_home, TOOL, &Wanted::Ours(&wanted));
            supervisor.extra_env.push((
                vw_agent_tools::powershell::ENV_REAL.to_owned(),
                real.clone().into_os_string(),
            ));
            format!(
                "PowerShell in a job: {} runs {} ({config})",
                copy.display(),
                real.display()
            )
        }
        Err(why) => {
            let existing = std::fs::read_to_string(supervisor.vibe_home.join("config.toml"))
                .unwrap_or_default();
            let ours = ours_in(&existing, data_dir);
            let config = write_tool_config(&supervisor.vibe_home, TOOL, &Wanted::None(&ours));
            format!("PowerShell without our wrapper: {why} ({config})")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_bash::{Shell, Wanted, with_tool_shell};

    #[test]
    fn powershell_7_is_found_before_windows_powershell_on_path() {
        let v51 = r"C:\Windows\System32\WindowsPowerShell\v1.0";
        let v7 = r"C:\Program Files\PowerShell\7";
        let path = format!("{v51};{v7}");
        // Joined as `find_real` joins them, so the test holds on every OS.
        let files = [
            Path::new(v51).join("powershell.exe"),
            Path::new(v7).join("pwsh.exe"),
        ];
        let var = |name: &str| (name == "PATH").then(|| path.clone());
        let found = find_real(var, |candidate: &Path| {
            files.iter().any(|file| file == candidate)
        });
        assert_eq!(found.as_ref(), Some(&files[1]), "PowerShell 7 first");
        let only_51 = find_real(var, |candidate: &Path| files[0] == candidate);
        assert_eq!(only_51.as_ref(), Some(&files[0]));
        assert_eq!(find_real(var, |_: &Path| false), None);
    }

    #[test]
    fn the_copy_takes_the_real_programs_name() {
        let data = Path::new("data");
        assert_eq!(
            copy_path(data, Path::new("x/PowerShell.EXE")),
            Some(data.join("shell").join("powershell.exe"))
        );
        assert_eq!(
            copy_path(data, Path::new("x/pwsh.exe")),
            Some(data.join("shell").join("pwsh.exe"))
        );
        assert_eq!(copy_path(data, Path::new("x/cmd.exe")), None);
    }

    #[test]
    fn our_powershell_line_is_added_and_removed_beside_the_git_bash_table() {
        let data = Path::new("data");
        let ours = data.join("shell").join("pwsh.exe");
        let ours = ours.to_string_lossy();
        let start = "[tools.git_bash]\nshell = \"users\"\n";
        let Shell::Changed(added) = with_tool_shell(start, TOOL, &Wanted::Ours(&ours)) else {
            panic!("not added");
        };
        assert!(added.contains("[tools.powershell]"), "{added}");
        assert!(
            added.contains("[tools.git_bash]\nshell = \"users\""),
            "{added}"
        );
        assert_eq!(ours_in(&added, data), ours);
        let Shell::Changed(removed) =
            with_tool_shell(&added, TOOL, &Wanted::None(&ours_in(&added, data)))
        else {
            panic!("not removed");
        };
        assert!(!removed.contains(ours.as_ref()), "{removed}");
        let users = "[tools.powershell]\nshell = 'C:/mine/pwsh.exe'\n";
        assert_eq!(
            with_tool_shell(users, TOOL, &Wanted::None(&ours_in(users, data))),
            Shell::UsersOwn("C:/mine/pwsh.exe".to_owned())
        );
    }
}
