//! Reads Vibe's discovery separately from a chat's pinned catalog. Called by `market`;
//! depends on the server's Python environment and `PluginResolver`, never parses plugin manifests itself.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::process::Command;

use crate::core::Core;

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

    pub(crate) async fn discover_plugins(
        &self,
        project: Option<&Path>,
        session_id: Option<&str>,
    ) -> Result<Value, String> {
        let server = self.shared.supervisor.program();
        let python = self
            .shared
            .market
            .python
            .clone()
            .or_else(|| {
                server.parent().map(|bin| {
                    bin.join(if cfg!(windows) {
                        "python.exe"
                    } else {
                        "python"
                    })
                })
            })
            .ok_or("Vibe's plugin discovery is unavailable: no Python environment.")?;
        let cwd = project.map_or_else(|| self.shared.supervisor.work_dir(), Path::to_path_buf);
        let mut command = Command::new(python);
        command
            .arg("-I")
            .arg("-B")
            .arg("-c")
            .arg(include_str!("plugin_discovery.py"))
            .arg(&self.shared.instructions.own_home)
            .arg(&cwd)
            .env_clear()
            .env("VIBE_HOME", &self.shared.instructions.own_home)
            .current_dir(&cwd)
            .kill_on_drop(true);
        // The app has no console of its own: without this, Windows opens a console window for every read.
        #[cfg(windows)]
        command.creation_flags(0x0800_0000);
        let workspace_roots = session_id
            .and_then(|id| self.shared.store.view(id))
            .and_then(|view| view.state.pointer("/session/workspaceRoots").cloned())
            .unwrap_or_else(|| json!([]));
        command.arg(json!({"builtinRoots": self.shared.market.builtin_roots, "workspaceRoots": workspace_roots}).to_string());
        for key in ["PATH", "SYSTEMROOT", "TEMP", "TMP", "HOME", "USERPROFILE"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        let output = tokio::time::timeout(Duration::from_secs(30), command.output())
            .await
            .map_err(|_| "Vibe's plugin discovery timed out.".to_owned())?
            .map_err(|_| "Vibe's plugin discovery could not start.".to_owned())?;
        if !output.status.success() {
            return Err("Vibe's plugin discovery failed. The chat's pinned plugins are shown when available.".to_owned());
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|_| "Vibe's plugin discovery returned an unreadable reply.".to_owned())
    }
}
