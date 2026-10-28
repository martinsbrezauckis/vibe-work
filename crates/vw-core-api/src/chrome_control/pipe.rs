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

pub(super) struct Pipe {
    pub(super) child: Child,
    _executable: Option<vw_platform::checked_executable::CheckedExecutable>,
    input: Option<ChildStdin>,
    output: BufReader<tokio::io::Take<ChildStdout>>,
    id: u64,
    shared: Weak<Shared>,
}

impl Pipe {
    pub(super) async fn launch(
        package: &super::setup::ChromeControlConfig,
        dir: &Path,
        expected: blake3::Hash,
        secret: &vw_vault::SecretString,
        shared: Weak<Shared>,
    ) -> Result<Self, ()> {
        let (mut executable, helper) = super::setup::open_helper(package).map_err(|_| ())?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(executable.file(), &mut bytes).map_err(|_| ())?;
        super::setup::check_helper_pin(package, &bytes).map_err(|_| ())?;
        if blake3::hash(&bytes) != expected {
            return Err(());
        }
        #[cfg(test)]
        if let Some(after_check) = &package.after_helper_check {
            after_check(&helper);
        }
        drop(helper);
        let mut command = tokio::process::Command::new(executable.path());
        command
            .args(["owned", "--state-dir"])
            .arg(dir)
            .env_remove("CHROME_CONTROL_STATE_DIR")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000);
        let mut child = command.spawn().map_err(|_| ())?;
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
            return Err(());
        }
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut bytes = Vec::new();
                let _ = stderr.take(4096).read_to_end(&mut bytes).await;
                for code in [
                    "OWNER_CONFLICT",
                    "OWNER_IDENTITY_MISMATCH",
                    "OWNER_APPROVAL_REQUIRED",
                ] {
                    if bytes
                        .windows(code.len())
                        .any(|part| part == code.as_bytes())
                    {
                        // A closing process's stderr can arrive after its connection was cleared.
                        tracing::warn!(code, "Chrome Control helper reported an error");
                        break;
                    }
                }
            });
        }
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
            shared,
        };
        if pipe.initialize(secret).await.is_err() {
            // A refused helper is ended and reaped before the error returns: dropping it only starts the kill, and
            // Windows keeps a still-running helper's executable and files locked for whoever touches them next.
            let _ = pipe.child.kill().await;
            return Err(());
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
        self.input
            .as_mut()
            .ok_or(())?
            .write_all(&line)
            .await
            .map_err(|_| ())
    }

    pub(super) async fn rpc(&mut self, method: &str, params: Value) -> Result<Value, ()> {
        self.id += 1;
        let id = self.id;
        tokio::time::timeout(Duration::from_secs(35), async {
            self.send(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
                .await?;
            loop {
                self.output.get_mut().set_limit(1024 * 1024 + 2);
                let mut line = Vec::new();
                let n = self
                    .output
                    .read_until(b'\n', &mut line)
                    .await
                    .map_err(|_| ())?;
                if n == 0 || n > 1024 * 1024 + 1 || line.last() != Some(&b'\n') {
                    return Err(());
                }
                let value: Value = serde_json::from_slice(&line).map_err(|_| ())?;
                if value["id"] == id {
                    return value.get("result").cloned().ok_or(());
                }
                if value.get("id").is_some() {
                    return Err(());
                }
                super::owner::observe(&self.shared, &value).await?;
                if value["method"] == "notifications/owner/error" {
                    return Err(());
                }
            }
        })
        .await
        .map_err(|_| ())?
    }

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
