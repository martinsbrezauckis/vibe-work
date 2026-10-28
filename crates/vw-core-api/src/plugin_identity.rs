//! Manifest identities for approval checks. Called by plugin approvals; uses
//! the same bounded, read-only pinned Vibe resolver as Tools for every format. All folder reads run
//! inside the child deadline; no servers start here.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::error::{CoreError, ErrorCode};

const MAX_IDENTITY_BYTES: u64 = 1024 * 1024;

pub(crate) struct PluginIdentity {
    python: Option<PathBuf>,
    runtime_python: Mutex<Option<PathBuf>>,
}

fn error(problem: impl std::fmt::Display) -> CoreError {
    CoreError::new(
        ErrorCode::Internal,
        format!("The plugin's manifest name could not be checked: {problem}"),
    )
}

impl PluginIdentity {
    pub(crate) fn new(python: Option<PathBuf>) -> Self {
        Self {
            python,
            runtime_python: Mutex::new(None),
        }
    }

    /// The actual server path also covers a deferred install or a newly selected Vibe environment (on Windows also
    /// a uv or pipx launcher that has no interpreter beside it: [`vw_supervisor::server_python`]).
    pub(crate) fn set_program(&self, program: &Path) {
        *self
            .runtime_python
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = vw_supervisor::server_python(program);
    }

    /// The pinned Vibe's Python: the configured one, else the one next to the running server.
    pub(crate) fn python(&self) -> Option<PathBuf> {
        self.python.clone().or_else(|| {
            self.runtime_python
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        })
    }

    pub(crate) fn name(&self, folder: &Path, home: &Path) -> Result<Option<String>, CoreError> {
        let python = self
            .python()
            .ok_or_else(|| error("Vibe's Python environment is unavailable"))?;
        let code = concat!(
            include_str!("plugin_read_guard.py"),
            "\n",
            include_str!("plugin_identity.py")
        );
        let output = run_vibe_python(
            &python,
            code,
            &[folder],
            home,
            Duration::from_secs(30),
            MAX_IDENTITY_BYTES,
        )
        .map_err(error)?;
        let names: Vec<String> = serde_json::from_slice(&output).map_err(error)?;
        Ok((names.len() == 1).then(|| names[0].clone()))
    }
}

/// Runs `code` with Vibe's isolated Python (`-I -B`, a cleared environment with `VIBE_HOME` = `home`, nothing on
/// stdin, stderr dropped) and returns its stdout. Bounded: killed after `limit`, at most `max_bytes` of output.
///
/// # Errors
/// A plain reason when the process cannot start, times out, fails, or prints too much.
pub(crate) fn run_vibe_python(
    python: &Path,
    code: &str,
    args: &[&Path],
    home: &Path,
    limit: Duration,
    max_bytes: u64,
) -> Result<Vec<u8>, String> {
    let until = Instant::now() + limit;
    let mut command = Command::new(python);
    command
        .args(["-I", "-B", "-c", code])
        .args(args)
        .env_clear()
        .env("VIBE_HOME", home)
        .current_dir(home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // The program folders and LOCALAPPDATA are where Vibe looks for Git Bash on Windows when it is not on PATH
    // (`get_windows_bash_path`); the server inherits them, so a helper reading Vibe's shell-dependent defaults
    // (`vibe_tool_defaults.py`) must see the same shell. Unset elsewhere.
    for key in [
        "PATH",
        "SYSTEMROOT",
        "TEMP",
        "TMP",
        "HOME",
        "USERPROFILE",
        "PROGRAMFILES",
        "PROGRAMFILES(X86)",
        "LOCALAPPDATA",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("no output".to_owned());
    };
    let reader = std::thread::spawn(move || {
        let mut stdout = stdout;
        let mut bytes = Vec::new();
        (&mut stdout).take(max_bytes + 1).read_to_end(&mut bytes)?;
        // Keep draining even excess output so the child can exit without filling its pipe.
        io::copy(&mut stdout, &mut io::sink())?;
        if bytes.len() as u64 > max_bytes {
            return Err(io::Error::other("the output is too large"));
        }
        Ok(bytes)
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(10)),
            problem => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(match problem {
                    Err(problem) => problem.to_string(),
                    _ => "Vibe's Python helper timed out".to_owned(),
                });
            }
        }
    };
    let output = reader
        .join()
        .map_err(|_| "the output reader stopped".to_owned())?;
    if !status?.success() {
        return Err("Vibe's Python helper failed".to_owned());
    }
    output.map_err(|e| e.to_string())
}
