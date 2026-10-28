//! Trusted broker owner: stores the token and executable, serves local callers and launches gh.
//! Depends on the shared policy and IPC. Caller-provided environment and executable overrides are absent.

use crate::{Reply, Request, local, policy, wire};
use bstr::ByteSlice;
use interprocess::local_socket::{Listener, prelude::*};
use std::{
    io::{self, BufReader},
    path::Path,
    process::{Command, ExitStatus, Stdio},
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
        let directory = builder.tempdir()?;
        std::fs::create_dir(directory.path().join("config"))?;
        let (listener, endpoint) = local::bind(directory.path())?;
        Ok(Self {
            token,
            executable,
            listener,
            endpoint,
            directory,
        })
    }

    /// The non-secret endpoint name to put in the shim's environment.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Serve requests until the listener fails. Each request uses the same refusal policy.
    ///
    /// # Errors
    /// Returns listener or transport errors; process failures are returned as gh replies.
    pub fn serve(&self) -> io::Result<()> {
        loop {
            self.serve_one()?;
        }
    }

    /// Serve until the owner closes its control pipe. Active commands finish first.
    /// # Errors
    /// Listener or reply transport failure.
    pub fn serve_until(&self, stopped: &std::sync::atomic::AtomicBool) -> io::Result<()> {
        while !stopped.load(std::sync::atomic::Ordering::Acquire) {
            self.answer(Some(stopped))?;
        }
        Ok(())
    }

    /// Accept and answer one invocation, also useful to hosts that own the serving loop.
    ///
    /// # Errors
    /// Returns listener or reply transport errors. Invalid requests are answered with a refusal.
    pub fn serve_one(&self) -> io::Result<()> {
        self.answer(None)
    }

    fn answer(&self, stopped: Option<&std::sync::atomic::AtomicBool>) -> io::Result<()> {
        let stream = self.listener.accept()?;
        let mut stream = BufReader::new(stream);
        let mut reply = match wire::receive::<Request>(&mut stream) {
            Ok(_) if stopped.is_some_and(|s| s.load(std::sync::atomic::Ordering::Acquire)) => {
                Reply::refused("GitHub reader is not available.")
            }
            Ok(request) => self.run(&request),
            Err(_) => Reply::refused("invalid request; expected argv and cwd"),
        };
        reply.stdout = self.redact(&reply.stdout);
        reply.stderr = self.redact(&reply.stderr);
        wire::send(stream.get_mut(), &reply)
    }

    fn run(&self, request: &Request) -> Reply {
        if let Err(reason) = policy::validate(&request.argv) {
            return Reply::refused(&format!(
                "Vibe Work only lets Vibe read GitHub. Use the Push and Create PR buttons for changes.\n{reason}"
            ));
        }
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
            .stdin(Stdio::null());
        match child.output() {
            Ok(output) => Reply {
                exit_code: exit_code(output.status),
                stdout: output.stdout,
                stderr: output.stderr,
            },
            Err(error) => Reply::refused(&error.to_string()),
        }
    }

    fn redact(&self, bytes: &[u8]) -> Vec<u8> {
        bytes.replace(self.token.as_bytes(), b"[REDACTED]")
    }
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
