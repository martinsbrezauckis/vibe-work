//! The read-only GitHub broker's lifecycle: starting it with the token on its stdin, keeping it in step with
//! the latest gh answer, and stopping it (its endpoint file goes first).
use crate::GithubReadsStatus as Status;
use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;
use zeroize::Zeroizing;

use super::{GH_LATE, Reader, Running, State};

impl Reader {
    pub(super) fn stop(&self, state: &mut State) {
        let _ = std::fs::remove_file(self.endpoint_file());
        if let Some(mut running) = state.running.take() {
            drop(running.child.stdin.take());
            let identity = vw_procmon::identity(running.child.id());
            state.retired.push((running.child, identity));
        }
        state.view.enabled = false;
    }

    pub(super) fn start(&self, token: Zeroizing<String>) -> std::io::Result<Running> {
        let binary = self
            .binary
            .as_ref()
            .ok_or_else(|| std::io::Error::other("the GitHub reader executable is missing"))?;
        if self.shim_prepared.get().is_none() {
            return Err(std::io::Error::other(
                "the GitHub reader shim is unavailable",
            ));
        }
        let gh = self
            .real_gh
            .as_ref()
            .ok_or_else(|| std::io::Error::other("gh is not installed"))?;
        let mut command = vw_platform::process_command::hidden(binary);
        command
            .arg("gh-broker")
            .arg(gh)
            .env_clear()
            .envs(std::env::vars_os().filter(|(k, _)| {
                matches!(
                    k.to_str(),
                    Some(
                        "PATH"
                            | "HOME"
                            | "USERPROFILE"
                            | "SYSTEMROOT"
                            | "WINDIR"
                            | "TEMP"
                            | "TMP"
                            | "TMPDIR"
                            | "LANG"
                            | "LC_ALL"
                            | "HTTP_PROXY"
                            | "HTTPS_PROXY"
                            | "NO_PROXY"
                            | "ALL_PROXY"
                            | "http_proxy"
                            | "https_proxy"
                            | "no_proxy"
                            | "all_proxy"
                            | "SSL_CERT_FILE"
                            | "SSL_CERT_DIR"
                    )
                )
            }))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn()?;
        let result = (|| {
            let input = child
                .stdin
                .as_mut()
                .ok_or_else(|| std::io::Error::other("broker input unavailable"))?;
            writeln!(input, "{}", token.as_str())?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| std::io::Error::other("broker output unavailable"))?;
            let mut endpoint = String::new();
            BufReader::new(stdout).read_line(&mut endpoint)?;
            if endpoint.trim().is_empty() {
                return Err(std::io::Error::other("broker did not start"));
            }
            vw_platform::replace_private(&self.endpoint_file(), endpoint.trim().as_bytes())
                .map_err(std::io::Error::other)
        })();
        if let Err(error) = result {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        Ok(Running { child, token })
    }

    pub(super) fn maintain(&self, state: &mut State, token: std::io::Result<Zeroizing<String>>) {
        let token = match token {
            Ok(token) => token,
            Err(error) => {
                let was_on = state.running.is_some();
                self.stop(state);
                let late = error.kind() == std::io::ErrorKind::TimedOut;
                state.view.status = if was_on && !late {
                    Status::LoggedOut
                } else {
                    Status::Error
                };
                if late {
                    state.view.reason = Some(GH_LATE.to_owned());
                } else if !was_on {
                    state.view.reason = Some("gh did not provide a usable token".to_owned());
                }
                return;
            }
        };
        let alive = state
            .running
            .as_mut()
            .is_some_and(|r| r.child.try_wait().ok() == Some(None));
        if !alive
            || self.endpoint_file().with_extension("refresh").exists()
            || state
                .running
                .as_ref()
                .is_none_or(|r| r.token.as_str() != token.as_str())
        {
            self.stop(state);
            if let Ok(running) = self.start(token) {
                state.running = Some(running);
            } else {
                state.view.status = Status::Error;
                state.view.reason = Some("the reader process could not be started".to_owned());
            }
        }
        state.view.enabled = state.running.is_some();
        if state.view.enabled {
            state.view.status = Status::On;
        }
    }
}
