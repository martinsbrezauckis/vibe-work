//! Shared entry points for the standalone executables and the app's shipped tools binary.
//! Depends on the broker and wire protocol. Only the server reads a credential, over stdin.
use crate::{Broker, ENDPOINT_ENV, Request, call};
use std::io::{self, BufRead, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use zeroize::Zeroizing;

/// The only commands that may run locally without an account or broker: the version and gh's own help.
#[must_use]
pub fn local_command(args: &[String]) -> bool {
    args == ["--version"]
        || args == ["version"]
        || args == ["--help"]
        || args == ["-h"]
        || args.first().is_some_and(|a| a == "help")
            && args.iter().skip(1).all(|a| {
                matches!(
                    a.as_str(),
                    "pr" | "issue"
                        | "run"
                        | "repo"
                        | "api"
                        | "help"
                        | "environment"
                        | "reference"
                        | "exit-codes"
                        | "formatting"
                        | "view"
                        | "list"
                        | "checks"
                        | "diff"
                )
            })
}

/// Run the credential-free client. A file reference lets an already running shell follow rotation.
/// # Errors
/// The broker is unavailable or its reply cannot be written.
pub fn shim() -> io::Result<i32> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if local_command(&argv) {
        let executable = std::env::var_os("VW_GH_LOCAL_EXECUTABLE").ok_or_else(|| {
            io::Error::other("GitHub CLI is not available: ask the user to install GitHub CLI and restart Vibe Work, or use the GitHub connector.")
        })?;
        let shim = std::env::current_exe()?;
        let config = shim
            .parent()
            .ok_or_else(|| io::Error::other("GitHub reader shim folder is missing"))?
            .join("local-config");
        if !config.is_dir() {
            return Err(io::Error::other(
                "GitHub reader local config is not available",
            ));
        }
        let mut child = Command::new(executable);
        child.args(&argv).env_clear().stdin(Stdio::null());
        for name in [
            "PATH",
            "HOME",
            "USERPROFILE",
            "SYSTEMROOT",
            "WINDIR",
            "TEMP",
            "TMP",
            "LANG",
        ] {
            if let Some(value) = std::env::var_os(name) {
                child.env(name, value);
            }
        }
        child.env("GH_CONFIG_DIR", config);
        child
            .env("GH_PAGER", "")
            .env("PAGER", "")
            .env("GH_PROMPT_DISABLED", "1");
        return Ok(child.status()?.code().unwrap_or(1));
    }
    let configured = std::env::var(ENDPOINT_ENV).map_err(|_| {
        io::Error::other("GitHub reader is not available (no broker endpoint configured)")
    })?;
    let endpoint = if let Some(path) = configured.strip_prefix("file:") {
        std::fs::read_to_string(path).map_err(|_| io::Error::other("GitHub reader is not available. GitHub reads are off. The user can turn them on in Settings > General."))?
    } else {
        configured.clone()
    };
    let reply = call(
        endpoint.trim(),
        &Request {
            argv,
            cwd: std::env::current_dir()?,
        },
    )?;
    if authentication_failed(reply.exit_code, &reply.stderr)
        && let Some(path) = configured.strip_prefix("file:")
    {
        let _ = std::fs::write(Path::new(path).with_extension("refresh"), b"");
    }
    io::stdout().lock().write_all(&reply.stdout)?;
    io::stderr().lock().write_all(&reply.stderr)?;
    Ok(reply.exit_code)
}

/// Start the broker with one token line on stdin, then stop accepting when stdin closes.
/// An invocation already running finishes before the server exits.
/// # Errors
/// Invalid credentials, executable, endpoint or transport.
pub fn server(executable: &Path) -> io::Result<()> {
    let mut token = Zeroizing::new(String::new());
    io::stdin().lock().read_line(&mut token)?;
    let broker = Broker::bind(token.trim_end_matches(['\n', '\r']).to_owned(), executable)?;
    println!("{}", broker.endpoint());
    io::stdout().flush()?;
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = stopped.clone();
    let endpoint = broker.endpoint().to_owned();
    std::thread::spawn(move || {
        let _ = io::stdin().read(&mut [0]);
        stop.store(true, Ordering::Release);
        let _ = call(
            &endpoint,
            &Request {
                argv: Vec::new(),
                cwd: std::env::temp_dir(),
            },
        );
    });
    broker.serve_until(&stopped)
}

// gh exits 1 on an HTTP 401 with a stored/revoked token, and 4 when no authentication exists.
// Other exit-1 results (missing repo, 403, failed checks) must not cause credential refresh.
fn authentication_failed(exit: i32, stderr: &[u8]) -> bool {
    exit == 4 || (exit == 1 && stderr.windows(8).any(|part| part == b"HTTP 401"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_authentication_errors_request_refresh() {
        assert!(super::authentication_failed(4, b"authentication required"));
        assert!(super::authentication_failed(
            1,
            b"gh: Bad credentials (HTTP 401)"
        ));
        assert!(super::authentication_failed(
            1,
            b"HTTP 401: Bad credentials"
        ));
        assert!(!super::authentication_failed(1, b"HTTP 403: Forbidden"));
        assert!(!super::authentication_failed(1, b"checks failed"));
        assert!(!super::authentication_failed(0, b"HTTP 401"));
    }
}
