//! Trusted broker owner: stores the token and executable, serves local callers and launches gh.
//! Depends on the shared policy and IPC. Caller-provided environment and executable overrides are absent.

use crate::{Reply, Request, local, policy, wire};
use bstr::ByteSlice;
use interprocess::local_socket::{Listener, Stream, prelude::*};
#[cfg(not(windows))]
use std::process::Command;
use std::{
    io::{self, BufReader, Read},
    path::Path,
    process::{Child, ExitStatus, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use tempfile::TempDir;
use zeroize::Zeroizing;

/// A broker with a private endpoint and isolated gh config, owned by the trusted caller.
/// It deliberately has no `Debug` or serialization implementation that could expose its token.
pub struct Broker {
    token: Zeroizing<String>,
    executable: std::path::PathBuf,
    listener: Listener,
    endpoint: String,
    directory: TempDir,
    limit: Duration,
}

/// How long one gh run may take before the broker stops it.
pub const TIME_LIMIT: Duration = Duration::from_secs(120);

/// Where the broker keeps its private directory. A Unix socket path holds about 104 bytes (`sun_path`; 108 on Linux),
/// so a long `TMPDIR` (a deep test or build folder) would make the endpoint unbindable: then `/tmp` is used. The
/// directory stays 0700 and the socket 0600 either way.
fn socket_base() -> std::path::PathBuf {
    let temp = std::env::temp_dir();
    #[cfg(unix)]
    {
        // The separator, "vw-gh-" and six random characters, then "/broker.sock".
        const SUFFIX: usize = "/vw-gh-XXXXXX/broker.sock".len();
        if temp.as_os_str().len() + SUFFIX > 100 {
            return std::path::PathBuf::from("/tmp");
        }
    }
    temp
}

impl Broker {
    /// Bind a user-only local endpoint and retain a token supplied directly by the app.
    /// The executable must be the absolute path of the real gh, outside the shim directory.
    ///
    /// # Errors
    /// Returns an error for an empty/invalid token, relative executable, or IPC setup failure.
    pub fn bind(token: String, executable: &Path) -> io::Result<Self> {
        let token = Zeroizing::new(token);
        if token.is_empty() || token.contains(['\0', '\n', '\r']) || !executable.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a token and an absolute real gh path are required",
            ));
        }
        let executable = executable.canonicalize()?;
        let mut builder = tempfile::Builder::new();
        builder.prefix("vw-gh-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(std::fs::Permissions::from_mode(0o700));
        }
        let directory = builder.tempdir_in(socket_base())?;
        std::fs::create_dir(directory.path().join("config"))?;
        let (listener, endpoint) = local::bind(directory.path())?;
        Ok(Self {
            token,
            executable,
            listener,
            endpoint,
            directory,
            limit: TIME_LIMIT,
        })
    }

    /// Use another limit for one gh run (default [`TIME_LIMIT`]); a run past it is stopped and refused.
    #[must_use]
    pub fn with_time_limit(mut self, limit: Duration) -> Self {
        self.limit = limit;
        self
    }

    /// The non-secret endpoint name to put in the shim's environment.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Serve requests until the listener fails. Each connection is answered on its own thread, so a slow gh run
    /// never holds other callers; each request uses the same refusal policy.
    ///
    /// # Errors
    /// Returns listener errors; process failures are returned as gh replies.
    pub fn serve(&self) -> io::Result<()> {
        std::thread::scope(|scope| {
            loop {
                let stream = self.listener.accept()?;
                // A caller that goes away before its reply only ends its own connection.
                scope.spawn(move || {
                    let _ = self.answer(stream, None);
                });
            }
        })
    }

    /// Serve until the owner closes its control pipe, each connection on its own thread.
    /// Active commands finish first.
    /// # Errors
    /// Listener failure.
    pub fn serve_until(&self, stopped: &AtomicBool) -> io::Result<()> {
        std::thread::scope(|scope| {
            while !stopped.load(Ordering::Acquire) {
                let stream = self.listener.accept()?;
                scope.spawn(move || {
                    let _ = self.answer(stream, Some(stopped));
                });
            }
            Ok(())
        })
    }

    /// Accept and answer one invocation, also useful to hosts that own the serving loop.
    ///
    /// # Errors
    /// Returns listener or reply transport errors. Invalid requests are answered with a refusal.
    pub fn serve_one(&self) -> io::Result<()> {
        let stream = self.listener.accept()?;
        self.answer(stream, None)
    }

    fn answer(&self, stream: Stream, stopped: Option<&AtomicBool>) -> io::Result<()> {
        let mut stream = BufReader::new(stream);
        let mut reply = match wire::receive::<Request>(&mut stream) {
            Ok(_) if stopped.is_some_and(|s| s.load(Ordering::Acquire)) => {
                Reply::refused(crate::entry::UNAVAILABLE)
            }
            Ok(request) => self.run(&request),
            Err(_) => Reply::refused(
                "The GitHub request was not understood. Check the command arguments and try again.",
            ),
        };
        reply.stdout = self.redact(&reply.stdout);
        reply.stderr = self.redact(&reply.stderr);
        wire::send(stream.get_mut(), &reply)
    }

    fn run(&self, request: &Request) -> Reply {
        if let Err(reason) = policy::validate(&request.argv) {
            return Reply::refused(&format!(
                "Not run: {reason}. Use a supported GitHub read command. {}",
                crate::remote_change_refusal!()
            ));
        }
        #[cfg(windows)]
        let mut child = vw_platform::process_command::hidden(&self.executable);
        #[cfg(not(windows))]
        let mut child = Command::new(&self.executable);
        child.env_clear();
        for key in [
            "PATH",
            "HOME",
            "USERPROFILE",
            "SYSTEMROOT",
            "WINDIR",
            "TEMP",
            "TMP",
            "TMPDIR",
            "LANG",
            "LC_ALL",
            // gh uses Go's HTTP proxy and system certificate overrides. These are the trusted
            // app environment, never request-supplied values from the credential-free shim.
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "NO_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "no_proxy",
            "all_proxy",
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
        ] {
            if let Some(value) = std::env::var_os(key) {
                child.env(key, value);
            }
        }
        child
            .args(&request.argv)
            .current_dir(&request.cwd)
            .env("GH_TOKEN", self.token.as_str())
            .env("GH_HOST", "github.com")
            .env("GH_CONFIG_DIR", self.directory.path().join("config"))
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("GH_NO_EXTENSION_UPDATE_NOTIFIER", "1")
            .env("GH_PAGER", "")
            .env("PAGER", "")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        match child.spawn() {
            Ok(child) => self.finish(child),
            Err(error) => Reply::refused(&error.to_string()),
        }
    }

    /// Collects a started gh run, stopping it once it passes the time limit.
    fn finish(&self, mut child: Child) -> Reply {
        // Both pipes are drained while gh runs, so a large reply never blocks it. The readers are detached: a
        // process gh started that still holds a pipe after gh is stopped cannot hold back the refusal.
        let stdout = child
            .stdout
            .take()
            .map(|pipe| std::thread::spawn(move || drain(pipe)));
        let stderr = child
            .stderr
            .take()
            .map(|pipe| std::thread::spawn(move || drain(pipe)));
        let deadline = Instant::now() + self.limit;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
        };
        match status {
            Some(status) => Reply {
                exit_code: exit_code(status),
                stdout: collect(stdout),
                stderr: collect(stderr),
            },
            None => Reply::refused(&format!(
                "Stopped: the GitHub command did not finish within {} seconds. Ask for less (a narrower query, \
                 --limit, a single page) and try again.",
                self.limit.as_secs()
            )),
        }
    }

    fn redact(&self, bytes: &[u8]) -> Vec<u8> {
        bytes.replace(self.token.as_bytes(), b"[REDACTED]")
    }
}

fn drain(mut pipe: impl Read) -> Vec<u8> {
    let mut bytes = Vec::new();
    let _ = pipe.read_to_end(&mut bytes);
    bytes
}

fn collect(reader: Option<std::thread::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    reader.and_then(|r| r.join().ok()).unwrap_or_default()
}

fn exit_code(status: ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status
            .code()
            .unwrap_or_else(|| status.signal().map_or(1, |signal| 128 + signal))
    }
    #[cfg(windows)]
    {
        status.code().unwrap_or(1)
    }
}
