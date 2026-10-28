//! Reads Vibe's discovery separately from a chat's pinned catalog. Called by `market`;
//! depends on the server's Python environment and `PluginResolver`, never parses plugin manifests itself.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::core::Core;

const MAX_INVENTORY_BYTES: usize = 4 * 1024 * 1024;

async fn inventory_output(command: &mut Command, deadline: Duration) -> Result<Vec<u8>, String> {
    let until = tokio::time::Instant::now() + deadline;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "Vibe's plugin discovery could not start.".to_owned())?;
    let mut stdout = child.stdout.take().ok_or("No plugin discovery output.")?;
    let read = async move {
        let mut bytes = Vec::new();
        let mut overflow = false;
        let mut buffer = vec![0; 8192];
        loop {
            let count = stdout.read(&mut buffer).await?;
            if count == 0 {
                break;
            }
            if !overflow && bytes.len() + count <= MAX_INVENTORY_BYTES {
                bytes.extend_from_slice(&buffer[..count]);
            } else {
                // Drain excess while the child runs, retaining only the bounded prefix.
                overflow = true;
            }
        }
        Ok::<_, std::io::Error>((bytes, overflow))
    };
    let result = tokio::time::timeout_at(until, async {
        let ((bytes, overflow), status) = tokio::try_join!(read, child.wait())
            .map_err(|_| "Vibe's plugin discovery could not be read.".to_owned())?;
        if overflow {
            return Err("Vibe's plugin discovery output is too large.".to_owned());
        }
        if !status.success() {
            return Err("Vibe's plugin discovery failed. The chat's pinned plugins are shown when available.".to_owned());
        }
        Ok(bytes)
    }).await;
    if let Ok(result) = result {
        result
    } else {
        // Dropping the killed child lets Tokio reap it without extending the read deadline.
        let _ = child.start_kill();
        Err("Vibe's plugin discovery timed out.".to_owned())
    }
}

/// Additional built-ins come from the same roots passed to the Vibe server.
pub(crate) fn builtin_roots(supervisor: &vw_supervisor::SupervisorConfig) -> Vec<PathBuf> {
    supervisor
        .extra_args
        .windows(2)
        .filter(|args| args[0] == "--additional-builtin-plugin-root")
        .map(|args| PathBuf::from(&args[1]))
        .collect()
}

impl Core {
    /// The opened project, from a stored chat or a folder selected through the native picker.
    pub(crate) fn plugin_project(
        &self,
        session_id: Option<&str>,
        cwd: Option<&str>,
    ) -> Option<PathBuf> {
        session_id
            .and_then(|id| self.shared.store.view(id))
            .and_then(|v| {
                v.state
                    .pointer("/session/cwd")
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
            })
            .or_else(|| {
                cwd.map(PathBuf::from)
                    .filter(|p| self.shared.gate.is_picked_folder(p))
            })
    }

    /// The CLI home remains read-only even if someone opened its parent as a project.
    pub(crate) fn plugin_owned(&self, path: &Path, project: Option<&Path>, off: bool) -> bool {
        crate::market_local::managed(path, &self.shared.instructions.own_home, project, off)
            && self
                .shared
                .instructions
                .cli_home
                .as_ref()
                .is_none_or(|cli| {
                    let cli = cli.canonicalize().unwrap_or_else(|_| cli.clone());
                    !path
                        .canonicalize()
                        .unwrap_or_else(|_| path.to_path_buf())
                        .starts_with(cli)
                })
    }

    /// Sweeps `plugins/` (under the sweep lock), then reads the plugins through Vibe's resolver. Of the folders
    /// in `plugins/` only those the sweep kept are read: a folder copied in after it is left alone until the next
    /// sweep has checked it (reading its files could also keep Windows from moving it).
    pub(crate) async fn discover_plugins(
        &self,
        project: Option<&Path>,
        session_id: Option<&str>,
    ) -> Result<Value, String> {
        let kept = {
            let _held = self.shared.plugin_approvals.mutations.lock().await;
            self.shared.plugin_approvals.sweep_kept()
        }
        .map_err(|error| error.to_string())?;
        let server = self.shared.supervisor.program();
        let python = self
            .shared
            .market
            .python
            .clone()
            .or_else(|| vw_supervisor::server_python(&server))
            .ok_or("Vibe's plugin discovery is unavailable: no Python environment.")?;
        let cwd = project.map_or_else(|| self.shared.supervisor.work_dir(), Path::to_path_buf);
        let workspace_roots = session_id
            .and_then(|id| self.shared.store.view(id))
            .and_then(|view| view.state.pointer("/session/workspaceRoots").cloned())
            .unwrap_or_else(|| json!([]));
        let options = json!({
            "builtinRoots": self.shared.market.builtin_roots,
            "workspaceRoots": workspace_roots,
            "kept": kept,
        });
        let mut command =
            discovery_command(&python, &self.shared.instructions.own_home, &cwd, &options);
        let output = inventory_output(&mut command, Duration::from_secs(30)).await?;
        serde_json::from_slice(&output)
            .map_err(|_| "Vibe's plugin discovery returned an unreadable reply.".to_owned())
    }
}

/// The discovery process: the read guard and the discovery script on `python`, for Vibe home `home` and `cwd`.
fn discovery_command(python: &Path, home: &Path, cwd: &Path, options: &Value) -> Command {
    let mut command = Command::new(python);
    command
        .arg("-I")
        .arg("-B")
        .arg("-c")
        .arg(concat!(
            include_str!("plugin_read_guard.py"),
            "\n",
            include_str!("plugin_discovery.py")
        ))
        .arg(home)
        .arg(cwd)
        .arg(options.to_string())
        .env_clear()
        .env("VIBE_HOME", home)
        .current_dir(cwd)
        .kill_on_drop(true);
    // The app has no console of its own: without this, Windows opens a console window for every read.
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    for key in ["PATH", "SYSTEMROOT", "TEMP", "TMP", "HOME", "USERPROFILE"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
}

#[cfg(test)]
#[path = "plugin_reads_tests.rs"]
mod tests;
