//! Bounded MCP over the owned helper's stdin/stdout. Depends on tokio and `serde_json`;
//! called by the owner supervisor. EOF and malformed/oversize replies end the connection; a helper refused during
//! launch is ended and reaped before the error returns.
use crate::core::Shared;
use serde_json::{Value, json};
use std::{
    path::Path,
    process::Stdio,
    sync::{PoisonError, Weak},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout},
};

#[cfg(test)]
#[path = "pipe_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "pipe_identity_tests.rs"]
mod identity_tests;

/// Refusal codes the helper prints on stderr before it ends; the first one found names why it stopped.
const REFUSALS: [&str; 3] = [
    "OWNER_CONFLICT",
    "OWNER_IDENTITY_MISMATCH",
    "OWNER_APPROVAL_REQUIRED",
];

/// The helper's refusal when another helper holds this state folder's lock.
pub(super) const OWNER_CONFLICT: &str = "OWNER_CONFLICT";

/// Why a launch failed. `helper_code` is the helper's own refusal code (`REFUSALS`) when it printed one before it
/// ended; `None` for every other failure (not found, pin mismatch, spawn error, handshake error).
#[derive(Debug, Default)]
pub(super) struct LaunchError {
    pub(super) helper_code: Option<&'static str>,
}

impl From<()> for LaunchError {
    fn from((): ()) -> Self {
        Self::default()
    }
}

/// Reads the first 4 KiB of the helper's stderr and returns the first refusal code in it, logged as a warning.
async fn read_refusal(stderr: tokio::process::ChildStderr) -> Option<&'static str> {
    let mut bytes = Vec::new();
    let _ = stderr.take(4096).read_to_end(&mut bytes).await;
    let code = refusal_code(&bytes)?;
    // A closing process's stderr can arrive after its connection was cleared.
    tracing::warn!(code, "Chrome Control helper reported an error");
    Some(code)
}

fn refusal_code(bytes: &[u8]) -> Option<&'static str> {
    REFUSALS.into_iter().find(|code| {
        bytes
            .windows(code.len())
            .any(|part| part == code.as_bytes())
    })
}

pub(super) struct Pipe {
    pub(super) child: Child,
    _executable: Option<vw_platform::checked_executable::CheckedExecutable>,
    input: Option<ChildStdin>,
    output: BufReader<tokio::io::Take<ChildStdout>>,
    id: u64,
    read_buffer: Vec<u8>,
    shared: Weak<Shared>,
    /// Requests whose JSON-RPC error answer is a normal outcome (`begin_tolerant`): an older helper answers an
    /// enrollment request it does not know with "method not found", which must not end the connection.
    tolerant: std::collections::HashSet<u64>,
}

impl Pipe {
    pub(super) async fn launch(
        package: &super::setup::ChromeControlConfig,
        dir: &Path,
        expected: blake3::Hash,
        secret: &vw_vault::SecretString,
        shared: Weak<Shared>,
    ) -> Result<Self, LaunchError> {
        let (mut executable, helper) = super::setup::open_helper(package).map_err(|reason| {
            tracing::debug!(%reason, "Chrome Control helper could not be opened");
        })?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(executable.file(), &mut bytes).map_err(|_| ())?;
        super::setup::check_helper_pin(package, &bytes).map_err(|_| ())?;
        if blake3::hash(&bytes) != expected {
            return Err(LaunchError::default());
        }
        #[cfg(test)]
        if let Some(after_check) = &package.after_helper_check {
            after_check(&helper);
        }
        drop(helper);
        let mut command = vw_platform::process_command::hidden_tokio(executable.path());
        command
            .args(["owned", "--state-dir"])
            .arg(dir)
            .env_remove("CHROME_CONTROL_STATE_DIR")
            .env_remove("CHROME_CONTROL_SAFARI_GROUP")
            .env_remove("CHROME_CONTROL_SAFARI_EXTENSION_ID")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(all(feature = "your-safari", target_os = "macos"))]
        super::safari_package::configure(&mut command, &shared).await;
        let mut child = command.spawn().map_err(|error| {
            tracing::debug!(%error, "Chrome Control helper could not be launched");
        })?;
        #[cfg(target_os = "macos")]
        if child
            .id()
            .zip(package.helper_cdhash.as_deref())
            .is_none_or(|(pid, hash)| {
                vw_platform::checked_executable::verify_child_cdhash(pid, hash).is_err()
            })
        {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(LaunchError::default());
        }
        let refusal = child
            .stderr
            .take()
            .map(|stderr| tokio::spawn(read_refusal(stderr)));
        let input = child.stdin.take().ok_or(())?;
        let output = child.stdout.take().ok_or(())?;
        let mut pipe = Self {
            child,
            _executable: if cfg!(target_os = "linux") {
                Some(executable)
            } else {
                drop(executable);
                None
            },
            input: Some(input),
            output: BufReader::new(output.take(1024 * 1024 + 2)),
            id: 0,
            read_buffer: Vec::new(),
            shared,
            tolerant: std::collections::HashSet::new(),
        };
        if pipe.initialize(secret).await.is_err() {
            // A refused helper is ended and reaped before the error returns: dropping it only starts the kill, and
            // Windows keeps a still-running helper's executable and files locked for whoever touches them next.
            let _ = pipe.child.kill().await;
            // The helper's stderr ends with it; its refusal code is read before the error returns so the caller can
            // name the cause (bounded: a process the helper started could still hold the stream open).
            let helper_code = match refusal {
                Some(task) => tokio::time::timeout(Duration::from_secs(1), task)
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .flatten(),
                None => None,
            };
            return Err(LaunchError { helper_code });
        }
        Ok(pipe)
    }

    async fn initialize(&mut self, secret: &vw_vault::SecretString) -> Result<(), ()> {
        // Keep the only encoded handshake in zeroizing memory, never in a JSON value or an error.
        let line = zeroize::Zeroizing::new(format!(
            "{{\"protocol\":\"vibework-owned/1\",\"secret\":\"{}\"}}\n",
            secret.expose()
        ));
        self.input
            .as_mut()
            .ok_or(())?
            .write_all(line.as_bytes())
            .await
            .map_err(|_| ())?;
        self.rpc("initialize", json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"Vibe Work","version":env!("CARGO_PKG_VERSION")}})).await?;
        self.notify("notifications/initialized", json!({})).await?;
        let list = self.rpc("tools/list", json!({})).await?;
        let tools = list["tools"].as_array().ok_or(())?;
        if tools.len() != 3
            || super::owner::TOOLS.iter().any(|name| {
                !tools
                    .iter()
                    .any(|tool| tool["name"] == *name && tool["inputSchema"].is_object())
            })
        {
            return Err(());
        }
        if let Some(shared) = self.shared.upgrade() {
            shared
                .chrome_control
                .owner
                .tools
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone_from(tools);
        }
        Ok(())
    }

    pub(super) async fn notify(&mut self, method: &str, params: Value) -> Result<(), ()> {
        self.send(&json!({"jsonrpc":"2.0","method":method,"params":params}))
            .await
    }

    async fn send(&mut self, message: &Value) -> Result<(), ()> {
        let mut line = serde_json::to_vec(message).map_err(|_| ())?;
        if line.len() > 1024 * 1024 {
            return Err(());
        }
        line.push(b'\n');
        tokio::time::timeout(
            Duration::from_millis(500),
            self.input.as_mut().ok_or(())?.write_all(&line),
        )
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
    }

    pub(super) async fn begin(&mut self, method: &str, params: Value) -> Result<u64, ()> {
        self.id += 1;
        self.send(&json!({"jsonrpc":"2.0","id":self.id,"method":method,"params":params}))
            .await?;
        Ok(self.id)
    }

    /// Like [`Self::begin`], but a JSON-RPC error answer arrives as `{"rpcError": error}` instead of ending the
    /// connection. Only the owner's enrollment requests use it.
    pub(super) async fn begin_tolerant(&mut self, method: &str, params: Value) -> Result<u64, ()> {
        let id = self.begin(method, params).await?;
        self.tolerant.insert(id);
        Ok(id)
    }

    // The buffer lives across select cancellation, including a partially read line.
    pub(super) async fn next(&mut self) -> Result<(u64, Value), ()> {
        loop {
            self.output
                .get_mut()
                .set_limit((1024 * 1024 + 2 - self.read_buffer.len()) as u64);
            let n = self
                .output
                .read_until(b'\n', &mut self.read_buffer)
                .await
                .map_err(|_| ())?;
            if n == 0
                || self.read_buffer.len() > 1024 * 1024 + 1
                || self.read_buffer.last() != Some(&b'\n')
            {
                return Err(());
            }
            let line = std::mem::take(&mut self.read_buffer);
            let value: Value = serde_json::from_slice(&line).map_err(|_| ())?;
            if let Some(id) = value.get("id") {
                let id = id.as_u64().ok_or(())?;
                let tolerant = self.tolerant.remove(&id);
                if let Some(result) = value.get("result") {
                    return Ok((id, result.clone()));
                }
                return match value.get("error") {
                    Some(error) if tolerant => Ok((id, json!({"rpcError": error}))),
                    _ => Err(()),
                };
            }
            super::owner::observe(&self.shared, &value)?;
            if value["method"] == "notifications/owner/error" {
                return Err(());
            }
        }
    }

    pub(super) async fn rpc(&mut self, method: &str, params: Value) -> Result<Value, ()> {
        tokio::time::timeout(Duration::from_secs(35), async {
            let id = self.begin(method, params).await?;
            let (reply, value) = self.next().await?;
            if reply != id {
                return Err(());
            }
            Ok(value)
        })
        .await
        .map_err(|_| ())?
    }

    pub(super) async fn abort(mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }

    // Only the unix reply-identity test closes a pipe gracefully.
    #[cfg(all(test, unix))]
    pub(super) async fn close(mut self) {
        drop(self.input.take());
        if tokio::time::timeout(Duration::from_secs(2), self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.kill().await;
            let _ = self.child.wait().await;
        }
    }
}
