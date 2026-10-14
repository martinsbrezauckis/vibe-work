//! Starting an agent process that must outlive Vibe Work (class `agent`: "stays up until
//! someone asks to stop it, also across quit"). Vibe's own background processes die with Vibe
//! because they run under its PTY.
//!
//! How: the shell runs the agent's command line (Unix `/bin/sh -c`, Windows prefers PowerShell 7, falling
//! back to 5.1 when absent) with stdin from the null device and stdout + stderr appended to a user-only log file (a pipe to the
//! core would break the server with SIGPIPE / EPIPE once the core is gone). Unix: its own process group
//! (`process_group(0)`: safe API, no `pre_exec`, so no parent-death signal and no terminal), so a group stop
//! hits exactly its tree. Windows: `CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW`, and
//! `CREATE_BREAKAWAY_FROM_JOB` so a kill-on-close job the app may sit in does not take it down (retried without
//! when the job forbids breakaway: then it lives as long as that job, reported in the result).
//! A reaper thread waits on the child, so an exit while the app runs leaves no zombie and is reported.
//!
//! Depends on: std. Called by: vw-core-api (`agent_tools` `process_start` / `process_restart`), tests.
//! Invariant: the environment is exactly `env` (cleared first): the caller's allowlist, never the app's own.

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io;
#[cfg(windows)]
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
#[cfg(test)]
use std::sync::Condvar;
use std::sync::{Arc, Mutex, PoisonError};

/// What to start.
#[derive(Clone, Copy, Debug)]
pub struct AgentCommand<'a> {
    /// The shell command line as the agent wrote it.
    pub command: &'a str,
    /// Working folder.
    pub cwd: &'a Path,
    /// The complete environment.
    pub env: &'a [(OsString, OsString)],
    /// Output log (created user-only, appended to).
    pub log: &'a Path,
}

/// The exit status a reaper records: `None` while running, `Some(code)` after exit (`None` code = signal).
pub type ExitSlot = Arc<Mutex<Option<Option<i32>>>>;

/// A started process.
#[derive(Clone, Debug)]
pub struct Started {
    /// Its pid.
    pub pid: u32,
    /// Its process group (Unix: its own pid).
    pub group: Option<u32>,
    /// Filled by the reaper thread when it exits while this app runs.
    pub exit: ExitSlot,
    #[cfg(test)]
    exit_changed: Arc<Condvar>,
    /// Windows: false when the job the app runs in forbade breakaway (then it ends with that job).
    pub broke_away: bool,
    /// The shell executable actually used, also persisted in the core's process record.
    pub shell: &'static str,
}

impl Started {
    /// `Some(code)` once the process has exited (code `None` = ended by a signal).
    #[must_use]
    pub fn exited(&self) -> Option<Option<i32>> {
        *self.exit.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[cfg(test)]
    fn wait_for_exit(&self, timeout: std::time::Duration) -> bool {
        let state = self.exit.lock().unwrap_or_else(PoisonError::into_inner);
        let (state, _) = self
            .exit_changed
            .wait_timeout_while(state, timeout, |state| state.is_none())
            .unwrap_or_else(PoisonError::into_inner);
        state.is_some()
    }
}

#[cfg(test)]
fn record_exit(exit: &ExitSlot, changed: &Condvar, code: Option<i32>) {
    *exit.lock().unwrap_or_else(PoisonError::into_inner) = Some(code);
    changed.notify_all();
}

fn open_log(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(unix)]
fn command(spec: &AgentCommand<'_>, shell: &str) -> Command {
    use std::os::unix::process::CommandExt;
    let mut cmd = Command::new(shell);
    cmd.arg("-c").arg(spec.command).process_group(0);
    cmd
}

// Kept platform-independent so the Windows command line can be checked on Linux too.
#[cfg(any(test, windows))]
fn windows_argv(script: &Path) -> Vec<OsString> {
    [
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
    ]
    .into_iter()
    .map(OsString::from)
    .chain(std::iter::once(script.as_os_str().to_owned()))
    .collect()
}

#[cfg(any(test, windows))]
fn windows_script(command: &str) -> String {
    // Remove the transient script once PowerShell has loaded it. The reaper also removes it
    // after exit if this failed (for example, while a file indexer held it open).
    format!(
        "$PORT = $env:PORT\nRemove-Item -LiteralPath $PSCommandPath -Force -ErrorAction SilentlyContinue\n{command}\n"
    )
}

#[cfg(windows)]
fn command(shell: &str, script: &Path) -> Command {
    let mut cmd = Command::new(shell);
    cmd.args(windows_argv(script));
    cmd
}

#[cfg(windows)]
fn write_windows_script(spec: &AgentCommand<'_>) -> io::Result<std::path::PathBuf> {
    // The log's parent is the core's user-only output directory. A random name and exclusive
    // create avoid overwriting another process's script or following an existing file.
    let directory = spec.log.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "the output log has no parent directory",
        )
    })?;
    for _ in 0..8 {
        let name = crate::credential::new_credential()
            .map_err(|e| io::Error::other(format!("script name randomness failed: {e}")))?;
        let path = directory.join(format!("process-{name}.ps1"));
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        // Windows PowerShell 5.1 needs a BOM to decode UTF-8 source reliably.
        let result = file
            .write_all(b"\xef\xbb\xbf")
            .and_then(|()| file.write_all(windows_script(spec.command).as_bytes()))
            .and_then(|()| file.flush());
        drop(file);
        if let Err(e) = result {
            let _ = std::fs::remove_file(&path);
            return Err(e);
        }
        return Ok(path);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create a unique process script",
    ))
}

#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

fn prepared(
    spec: &AgentCommand<'_>,
    shell: &str,
    #[cfg(windows)] script: &Path,
) -> io::Result<Command> {
    let log = open_log(spec.log)?;
    let err = log.try_clone()?;
    #[cfg(unix)]
    let mut cmd = command(spec, shell);
    #[cfg(windows)]
    let mut cmd = command(shell, script);
    let mut env = spec.env.to_vec();
    crate::agent_env::isolate_credentials(&mut env);
    cmd.current_dir(spec.cwd)
        .env_clear()
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err));
    Ok(cmd)
}

/// Starts `spec` detached from this app (see the module doc).
///
/// # Errors
/// The OS error when the log file cannot be opened or the shell cannot be started.
pub fn start(spec: &AgentCommand<'_>) -> io::Result<Started> {
    start_shell(spec, if cfg!(windows) { "pwsh.exe" } else { "/bin/sh" })
}

#[cfg(windows)]
fn windows_child(
    spec: &AgentCommand<'_>,
    shell: &str,
    script: &Path,
) -> io::Result<(std::process::Child, bool)> {
    use std::os::windows::process::CommandExt;
    let base = CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW;
    match prepared(spec, shell, script)?
        .creation_flags(base | CREATE_BREAKAWAY_FROM_JOB)
        .spawn()
    {
        Ok(child) => Ok((child, true)),
        // ERROR_ACCESS_DENIED: the job we are in does not allow breakaway.
        Err(e) if e.raw_os_error() == Some(5) => Ok((
            prepared(spec, shell, script)?
                .creation_flags(base)
                .spawn()?,
            false,
        )),
        Err(e) => Err(e),
    }
}

fn start_shell(spec: &AgentCommand<'_>, preferred: &'static str) -> io::Result<Started> {
    #[cfg(windows)]
    let script = write_windows_script(spec)?;
    #[cfg(windows)]
    let (child, broke_away, shell) = {
        match windows_child(spec, preferred, &script) {
            Ok((child, broke_away)) => (child, broke_away, preferred),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                match windows_child(spec, "powershell.exe", &script) {
                    Ok((child, broke_away)) => (child, broke_away, "powershell.exe"),
                    Err(e) => {
                        let _ = std::fs::remove_file(&script);
                        return Err(e);
                    }
                }
            }
            Err(e) => {
                let _ = std::fs::remove_file(&script);
                return Err(e);
            }
        }
    };
    #[cfg(not(windows))]
    let (child, broke_away, shell) = (prepared(spec, preferred)?.spawn()?, true, preferred);
    let pid = child.id();
    let exit: ExitSlot = Arc::new(Mutex::new(None));
    #[cfg(test)]
    let exit_changed = Arc::new(Condvar::new());
    let slot = exit.clone();
    #[cfg(test)]
    let changed = exit_changed.clone();
    let mut child = child;
    // The reaper: a small thread per started process (at most a few per session). When the app exits the
    // thread ends with it and the process keeps running (reparented by the OS).
    let _ = std::thread::Builder::new()
        .name("vw-agent-reaper".to_owned())
        .stack_size(64 * 1024)
        .spawn(move || {
            let code = child.wait().ok().and_then(|status| status.code());
            #[cfg(windows)]
            let _ = std::fs::remove_file(script);
            #[cfg(test)]
            record_exit(&slot, &changed, code);
            #[cfg(not(test))]
            {
                *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(code);
            }
        });
    Ok(Started {
        pid,
        group: if cfg!(unix) { Some(pid) } else { None },
        exit,
        #[cfg(test)]
        exit_changed,
        broke_away,
        shell,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod long_command_tests {
    use super::{windows_argv, windows_script};
    use std::path::Path;

    #[test]
    fn windows_long_command_is_in_script_and_argv_stays_short() {
        let payload = format!(
            "Write-Output \"port=$PORT\" && Write-Output 'done'\n#{}",
            "x".repeat(11_000)
        );
        let script = windows_script(&payload);
        assert!(script.len() > 10_000);
        assert!(script.starts_with("$PORT = $env:PORT\n"));
        assert!(script.contains(&payload));
        let argv = windows_argv(Path::new("C:\\private output\\process-a.ps1"));
        assert_eq!(argv[5], "-File");
        assert_eq!(argv[6], "C:\\private output\\process-a.ps1");
        assert!(
            !argv
                .iter()
                .any(|arg| arg.to_string_lossy().contains("Write-Output"))
        );
        assert!(argv.iter().map(|arg| arg.len()).sum::<usize>() < 200);
    }

    #[cfg(unix)]
    #[test]
    fn unix_shell_accepts_command_longer_than_ten_kib() {
        use super::{AgentCommand, start};
        let command = format!("test \"$PORT\" = 28100 && true\n#{}", "x".repeat(11_000));
        let env = [("PORT".into(), "28100".into())];
        let spec = AgentCommand {
            command: &command,
            cwd: Path::new("/"),
            env: &env,
            log: Path::new("/dev/null"),
        };
        let started = start(&spec).unwrap();
        assert!(started.wait_for_exit(std::time::Duration::from_secs(30)));
        assert_eq!(started.exited(), Some(Some(0)));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod exit_wait_tests {
    #[cfg(unix)]
    use super::{AgentCommand, start};
    use super::{Started, record_exit};
    #[cfg(unix)]
    use std::path::Path;
    use std::sync::{Arc, Barrier, Condvar, Mutex};
    use std::time::Duration;

    fn started() -> Started {
        Started {
            pid: 0,
            group: None,
            exit: Arc::new(Mutex::new(None)),
            exit_changed: Arc::new(Condvar::new()),
            broke_away: false,
            shell: "test",
        }
    }

    #[test]
    fn exit_observed_before_wait_is_not_missed() {
        let started = started();
        record_exit(&started.exit, &started.exit_changed, Some(7));
        assert!(started.wait_for_exit(Duration::ZERO));
        assert_eq!(started.exited(), Some(Some(7)));
    }

    #[test]
    fn notification_wakes_concurrent_waiters() {
        let started = started();
        let ready = Barrier::new(3);
        std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                ready.wait();
                started.wait_for_exit(Duration::from_secs(5))
            });
            let second = scope.spawn(|| {
                ready.wait();
                started.wait_for_exit(Duration::from_secs(5))
            });
            ready.wait();
            record_exit(&started.exit, &started.exit_changed, None);
            assert!(first.join().unwrap());
            assert!(second.join().unwrap());
        });
        assert_eq!(started.exited(), Some(None));
    }

    #[test]
    fn timeout_does_not_hide_later_exit() {
        let started = started();
        assert!(!started.wait_for_exit(Duration::ZERO));
        record_exit(&started.exit, &started.exit_changed, Some(0));
        assert!(started.wait_for_exit(Duration::ZERO));
        assert_eq!(started.exited(), Some(Some(0)));
    }

    #[cfg(unix)]
    #[test]
    fn shell_exit_is_recorded_by_reaper() {
        let spec = AgentCommand {
            command: "exit 7",
            cwd: Path::new("/"),
            env: &[],
            log: Path::new("/dev/null"),
        };
        let started = start(&spec).unwrap();
        assert!(started.wait_for_exit(Duration::from_secs(30)));
        assert_eq!(started.exited(), Some(Some(7)));
    }
}

#[cfg(all(test, windows))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod windows_tests {
    use super::{AgentCommand, Started, start, start_shell};
    use std::time::Duration;

    fn run(command: &str, preferred: Option<&'static str>) -> (Started, String) {
        let root = std::env::temp_dir().join(format!(
            "vw-wx1-shell-{}-{}",
            std::process::id(),
            crate::credential::new_credential().unwrap()
        ));
        std::fs::create_dir(&root).unwrap();
        let log = root.join("output.log");
        let mut env = vec![("PORT".into(), "41234".into())];
        for name in ["PATH", "SystemRoot", "TEMP", "TMP", "USERPROFILE"] {
            if let Some(value) = std::env::var_os(name) {
                env.push((name.into(), value));
            }
        }
        let spec = AgentCommand {
            command,
            cwd: &root,
            env: &env,
            log: &log,
        };
        let started = preferred
            .map_or_else(|| start(&spec), |shell| start_shell(&spec, shell))
            .unwrap();
        // PowerShell startup can be slow on loaded runners; wake as soon as the reaper records exit.
        assert!(
            started.wait_for_exit(Duration::from_secs(90)),
            "shell did not exit: {started:?}"
        );
        let output = std::fs::read_to_string(&log).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        (started, output)
    }

    #[test]
    fn pwsh_defines_port_and_supports_and_chain() {
        let (started, output) = run(
            "Write-Output \"port=$PORT env=$env:PORT\" && Write-Output 'chain-ok'",
            None,
        );
        assert_eq!(
            started.shell, "pwsh.exe",
            "PowerShell 7 must be on PATH to run this test"
        );
        assert_eq!(started.exited(), Some(Some(0)), "{output}");
        assert!(output.contains("port=41234 env=41234"), "{output}");
        assert!(output.contains("chain-ok"), "{output}");
    }

    #[test]
    fn absent_pwsh_falls_back_to_51_and_defines_port() {
        let (started, output) = run(
            "Write-Output \"port=$PORT env=$env:PORT major=$($PSVersionTable.PSVersion.Major)\"",
            Some("vw-wx1-absent-pwsh.exe"),
        );
        assert_eq!(started.shell, "powershell.exe");
        assert_eq!(started.exited(), Some(Some(0)), "{output}");
        assert!(output.contains("port=41234 env=41234 major=5"), "{output}");
    }
}
