//! Asking the user's gh: finding it (the app's one gh lookup), running one bounded `gh` call in its own process group or job,
//! and reading its sign-in state and token.
use crate::GithubReadsStatus as Status;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use zeroize::Zeroizing;

use super::{GH_LATE, GH_LIMIT, Reader};

/// gh's process while a call waits for it: dropping the call (its command was cancelled, the runtime shut down)
/// before gh answered stops gh's whole group or job, as the timeout does. On Unix `KillOnDrop` alone kills only
/// gh's own pid.
struct GhCall(Box<dyn process_wrap::tokio::ChildWrapper>, bool);

impl Drop for GhCall {
    fn drop(&mut self) {
        if !self.1 {
            let _ = self.0.start_kill();
        }
    }
}

/// Runs `gh <args>` without input. A call still running after [`GH_LIMIT`] is killed and reported as `TimedOut`.
///
/// gh runs as the leader of its own process group (Unix) or in a kill-on-close Job Object (Windows), as the
/// supervisor runs the server, so the timeout, and a call dropped before gh answered, stop every process gh
/// started that stays in that group or job (a credential helper, a pager), not only gh itself. On Unix a process
/// that leaves the group on purpose (`setsid`, a daemon) is out of reach of a group kill: only a cgroup or a
/// subreaper could follow it, and gh's own keyring access starts none (go-keyring: a D-Bus call on Linux, the
/// `security` tool as a plain child on macOS). gh is the user's own program: this is cleanup, not a containment
/// boundary.
async fn gh(program: &Path, args: &[&str]) -> std::io::Result<std::process::Output> {
    use process_wrap::tokio::KillOnDrop;
    use tokio::io::AsyncReadExt;
    let mut command = vw_platform::process_command::hidden_wrap(program, |cmd| {
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
    });
    #[cfg(unix)]
    command.wrap(process_wrap::tokio::ProcessGroup::leader());
    #[cfg(windows)]
    command.wrap(process_wrap::tokio::JobObject);
    command.wrap(KillOnDrop);
    let mut child = GhCall(command.spawn()?, false);
    let mut stdout = child.0.stdout().take();
    let mut stderr = child.0.stderr().take();
    let read = async {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let read_out = async {
            match stdout.as_mut() {
                Some(pipe) => pipe.read_to_end(&mut out).await.map(|_| ()),
                None => Ok(()),
            }
        };
        let read_err = async {
            match stderr.as_mut() {
                Some(pipe) => pipe.read_to_end(&mut err).await.map(|_| ()),
                None => Ok(()),
            }
        };
        // gh itself (the leader): its exit and the end of both pipes complete the call.
        let (status, (), ()) = tokio::try_join!(child.0.inner_mut().wait(), read_out, read_err)?;
        Ok(std::process::Output {
            status,
            stdout: out,
            stderr: err,
        })
    };
    let answered = tokio::time::timeout(GH_LIMIT, read).await;
    let Ok(output) = answered else {
        // Stop the whole tree (group or job), then reap it briefly so no zombie is left behind.
        let _ = child.0.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(2), child.0.wait()).await;
        return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, GH_LATE));
    };
    // gh answered (its exit and the end of both pipes): a process it left running on purpose stays, as before.
    child.1 = output.is_ok();
    output
}

/// gh for GitHub reads: the app's one gh lookup ([`vw_git::gh::locate`]), resolved to the real file.
pub(super) fn discover() -> Option<PathBuf> {
    discover_with(|key| std::env::var_os(key))
}

/// [`discover`] with the environment given (tests).
pub(super) fn discover_with(env: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    vw_git::gh::locate_with(vw_git::gh::InstallLayout::current(), env)
        .and_then(|p| p.canonicalize().ok())
}

// Schema: cli/cli pkg/cmd/auth/status/status.go (hosts -> accounts, with state, active and login).
// JSON status can exit zero even when authentication failed, so exit code alone never signs an account in.
fn structured_login(bytes: &[u8]) -> Option<(Status, Option<String>)> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let hosts = value.get("hosts")?.as_object()?;
    let Some(accounts) = hosts.get("github.com") else {
        return Some((
            if hosts.is_empty() {
                Status::SignedOut
            } else {
                Status::Enterprise
            },
            None,
        ));
    };
    let login = accounts
        .as_array()?
        .iter()
        .find(|entry| entry["active"] == true && entry["state"] == "success")
        .and_then(|entry| entry["login"].as_str())
        .filter(|login| !login.is_empty())
        .map(str::to_owned);
    Some((
        if login.is_some() {
            Status::SignedIn
        } else {
            Status::SignedOut
        },
        login,
    ))
}

impl Reader {
    /// `Err(GH_LATE)` when gh did not answer within [`GH_LIMIT`].
    pub(super) async fn login(&self) -> Result<(Status, Option<String>), &'static str> {
        let Some(program) = self.real_gh.as_ref().filter(|p| p.is_file()) else {
            return Ok((Status::Missing, None));
        };
        let output = match gh(
            program,
            &[
                "auth",
                "status",
                "--hostname",
                "github.com",
                "--json",
                "hosts",
            ],
        )
        .await
        {
            Ok(output) => output,
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => return Err(GH_LATE),
            Err(_) => return Ok((Status::SignedOut, None)),
        };
        if let Some(status) = structured_login(&output.stdout) {
            return Ok(status);
        }
        // Older gh versions reject --json. Preserve their existing text path without a version probe.
        let output = if output.status.success() {
            output
        } else {
            match gh(program, &["auth", "status", "--hostname", "github.com"]).await {
                Ok(output) => output,
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => return Err(GH_LATE),
                Err(_) => return Ok((Status::SignedOut, None)),
            }
        };
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if !text.contains("github.com") && output.status.success() {
            return Ok((Status::Enterprise, None));
        }
        if !output.status.success() {
            return Ok((Status::SignedOut, None));
        }
        let login = text
            .lines()
            .find(|line| {
                line.contains("github.com") && (line.contains("account ") || line.contains(" as "))
            })
            .and_then(|line| {
                line.split_once("account ")
                    .or_else(|| line.split_once(" as "))
            })
            .and_then(|(_, rest)| rest.split_whitespace().next())
            .map(str::to_owned);
        if login.is_none() {
            return Ok((Status::SignedOut, None));
        }
        Ok((Status::SignedIn, login))
    }

    pub(super) async fn token(&self) -> std::io::Result<Zeroizing<String>> {
        let program = self
            .real_gh
            .as_ref()
            .ok_or_else(|| std::io::Error::other("gh is not installed"))?;
        let output = gh(program, &["auth", "token", "--hostname", "github.com"]).await?;
        let bytes = Zeroizing::new(output.stdout);
        if !output.status.success() {
            return Err(std::io::Error::other("gh is not signed in"));
        }
        let token = Zeroizing::new(String::from_utf8_lossy(&bytes).trim().to_owned());
        if token.is_empty() || token.contains(['\n', '\r', '\0']) {
            return Err(std::io::Error::other("gh returned no usable token"));
        }
        Ok(token)
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "github_reads_tests.rs"]
mod tests;
