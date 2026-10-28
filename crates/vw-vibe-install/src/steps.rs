//! Runs one uv step of the install the way the server runs: whole process tree contained (process
//! group on Unix, kill-on-close Job Object on Windows, no console window), an allowlisted environment, a time
//! limit, cancellation, and the last part of the output kept for the screen and the log.
//!
//! Environment: starts empty; copies only the supervisor's allowlist (`vw_supervisor::INHERITED`: PATH, the
//! Windows profile names, proxy and CA settings), never a loader/runtime hijack name (`is_hijack_var`), then the
//! step's own `UV_*` names. So a user's `UV_INDEX_URL`, `PIP_*` or `uv.toml` cannot redirect the install
//! (`UV_NO_CONFIG=1`), while their proxy and certificate settings still apply.
//!
//! Depends on: process-wrap, tokio, vw-supervisor (`INHERITED`, `is_hijack_var`). Called by: [`crate::manager`].
//! Invariant: when `run` returns, no process of the step is alive (killed on cancel, timeout or drop).
//! Newly written executables get a short, bounded retry only while an open write handle prevents spawning.

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use process_wrap::tokio::{CommandWrap, KillOnDrop};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{mpsc, watch};

/// Characters of output kept per step.
pub const TAIL_CHARS: usize = 6_000;

/// Starts an executable, retrying only while it is busy with an open write handle.
///
/// At most five attempts, with delays of 20, 40, 80 and 160 ms. Used by install steps and existing-install
/// probes; `spawn` keeps the command's environment and process containment on every attempt.
///
/// # Errors
/// Returns the original spawn error immediately unless it is [`std::io::ErrorKind::ExecutableFileBusy`];
/// returns the last busy error when all four retries are exhausted.
pub async fn spawn_with_retry<T>(
    mut spawn: impl FnMut() -> std::io::Result<T>,
) -> std::io::Result<T> {
    let mut retries = 0;
    loop {
        match spawn() {
            // Linux can still see a writer after an executable was installed. Stop waiting once the handle
            // closes, or after the bounded delay; unrelated spawn failures must retain their normal behavior.
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy && retries < 4 => {
                tokio::time::sleep(Duration::from_millis(20 << retries)).await;
                retries += 1;
            }
            result => return result,
        }
    }
}

/// One command of the install.
#[derive(Debug, Clone)]
pub struct Step {
    /// Shown name ("Install Python 3.14.8").
    pub name: String,
    /// The executable (absolute).
    pub program: PathBuf,
    /// Its arguments.
    pub args: Vec<OsString>,
    /// Longest it may run.
    pub limit: Duration,
}

impl Step {
    /// The command line as shown in the confirmation, with `folder` written as `<install folder>`.
    #[must_use]
    pub fn display(&self, folder: &Path) -> String {
        let short = |s: &OsStr| {
            let text = s.to_string_lossy().into_owned();
            text.replace(&*folder.to_string_lossy(), "<install folder>")
        };
        std::iter::once(short(self.program.as_os_str()))
            .chain(self.args.iter().map(|a| short(a)))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Why a step did not succeed.
#[derive(Debug, thiserror::Error)]
pub enum StepError {
    /// It ran longer than its limit (killed).
    #[error("it did not finish within {0:?}")]
    Timeout(Duration),
    /// The user cancelled (killed).
    #[error("cancelled")]
    Cancelled,
    /// It exited with an error.
    #[error("it exited with code {0:?}")]
    Failed(Option<i32>),
    /// It could not start.
    #[error("it could not start: {0}")]
    Spawn(std::io::Error),
}

/// The environment of every step: the allowlisted parent names plus `own`.
#[must_use]
pub fn environment(
    parent: impl IntoIterator<Item = (OsString, OsString)>,
    own: &[(String, OsString)],
) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = parent
        .into_iter()
        .filter(|(name, _)| {
            let text = name.to_string_lossy();
            let allowed = if cfg!(windows) {
                vw_supervisor::INHERITED
                    .iter()
                    .any(|n| n.eq_ignore_ascii_case(&text))
            } else {
                vw_supervisor::INHERITED.contains(&text.as_ref())
            };
            allowed && !vw_supervisor::is_hijack_var(&text)
        })
        .collect();
    for (name, value) in own {
        env.retain(|(n, _)| !n.to_string_lossy().eq_ignore_ascii_case(name));
        env.push((OsString::from(name), value.clone()));
    }
    env
}

/// Keeps the last [`TAIL_CHARS`] characters of the output.
#[derive(Debug, Default)]
pub struct Tail(VecDeque<char>);

impl Tail {
    /// Adds one line.
    pub fn push_line(&mut self, line: &str) {
        self.0.extend(line.chars());
        self.0.push_back('\n');
        while self.0.len() > TAIL_CHARS {
            self.0.pop_front();
        }
    }

    /// The kept text.
    #[must_use]
    pub fn text(&self) -> String {
        self.0.iter().collect()
    }
}

/// Runs `step` in `cwd` with `env`; `on_line` gets every output line (stdout and stderr). Returns the output
/// tail on success.
///
/// # Errors
/// [`StepError`]; the tail of a failed step is in `tail`.
pub async fn run(
    step: &Step,
    env: &[(OsString, OsString)],
    cwd: &Path,
    mut cancel: watch::Receiver<bool>,
    tail: &mut Tail,
    mut on_line: impl FnMut(&str),
) -> Result<(), StepError> {
    let mut command = CommandWrap::with_new(&step.program, |cmd| {
        cmd.args(&step.args)
            .env_clear()
            .envs(env.iter().map(|(k, v)| (k.as_os_str(), v.as_os_str())))
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
    });
    #[cfg(unix)]
    command.wrap(process_wrap::tokio::ProcessGroup::leader());
    #[cfg(windows)]
    {
        command.wrap(process_wrap::tokio::CreationFlags(
            windows::Win32::System::Threading::CREATE_NO_WINDOW,
        ));
        command.wrap(process_wrap::tokio::JobObject);
    }
    command.wrap(KillOnDrop);
    let mut child = spawn_with_retry(|| command.spawn())
        .await
        .map_err(StepError::Spawn)?;
    let (lines_tx, mut lines) = mpsc::unbounded_channel::<String>();
    if let Some(out) = child.stdout().take() {
        forward(BufReader::new(out), lines_tx.clone());
    }
    if let Some(err) = child.stderr().take() {
        forward(BufReader::new(err), lines_tx.clone());
    }
    drop(lines_tx);
    let deadline = tokio::time::sleep(step.limit);
    tokio::pin!(deadline);
    let mut lines_open = true;
    let outcome = loop {
        tokio::select! {
            line = lines.recv(), if lines_open => match line {
                Some(line) => {
                    tail.push_line(&line);
                    on_line(&line);
                }
                None => lines_open = false,
            },
            status = child.inner_mut().wait() => {
                break match status {
                    Ok(status) if status.success() => Ok(()),
                    Ok(status) => Err(StepError::Failed(status.code())),
                    Err(error) => Err(StepError::Spawn(error)),
                };
            }
            () = &mut deadline => break Err(StepError::Timeout(step.limit)),
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    break Err(StepError::Cancelled);
                }
            }
        }
    };
    // The leader is gone or must go: end the whole tree, reap, then take the rest of the output.
    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;
    while let Ok(Some(line)) = tokio::time::timeout(Duration::from_millis(200), lines.recv()).await
    {
        tail.push_line(&line);
        on_line(&line);
    }
    outcome
}

fn forward<R: tokio::io::AsyncRead + Unpin + Send + 'static>(
    reader: BufReader<R>,
    to: mpsc::UnboundedSender<String>,
) {
    tokio::spawn(async move {
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if to.send(line).is_err() {
                return;
            }
        }
    });
}
