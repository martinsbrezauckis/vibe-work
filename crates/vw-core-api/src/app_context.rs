//! Measured machine facts for chats running inside Vibe Work.
//! Uses the process tool's filtered environment; dispatch adds bounded context before each model turn.
//! Version probes run once per app run, without a provider key or a package install.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::process::Command;
use tokio::sync::OnceCell;
use vw_supervisor::SupervisorConfig;

use crate::core::Shared;

pub(crate) struct Context {
    enabled: bool,
    env: Vec<(OsString, OsString)>,
    cwd: PathBuf,
    facts: OnceCell<String>,
}

impl Context {
    pub(crate) fn new(server: &SupervisorConfig, config: &crate::CoreConfig) -> Self {
        let enabled = std::fs::read_to_string(server.vibe_home.join("AGENTS.md"))
            .is_ok_and(|text| crate::working_note::is_ours(&text));
        let env = config.agent_tools.as_ref().map_or_else(
            || {
                crate::AgentToolsConfig::process_env_from(
                    server
                        .parent_env
                        .clone()
                        .unwrap_or_else(|| std::env::vars_os().collect()),
                    server.home.as_deref(),
                )
            },
            |tools| tools.process_env.clone(),
        );
        Self {
            enabled,
            env,
            cwd: server.cwd.clone(),
            facts: OnceCell::new(),
        }
    }

    async fn check(&self, shell: &str, script: &str) -> &'static str {
        let mut command = Command::new(shell);
        #[cfg(windows)]
        {
            command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
            command.creation_flags(0x0800_0000);
        }
        #[cfg(not(windows))]
        command.args(["-c", script]);
        command
            .env_clear()
            .envs(self.env.clone())
            .current_dir(&self.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        match tokio::time::timeout(Duration::from_secs(3), command.status()).await {
            Ok(Ok(status)) if status.success() => "runs",
            Ok(_) => "unavailable",
            Err(_) => "not checked (timed out)",
        }
    }

    async fn probe(&self, shell: &str, program: &str) -> &'static str {
        #[cfg(windows)]
        let script = format!(
            "$ErrorActionPreference='Stop'; & {program} --version; if ($LASTEXITCODE) {{ exit $LASTEXITCODE }}"
        );
        #[cfg(not(windows))]
        let script = format!("{program} --version");
        self.check(shell, &script).await
    }

    async fn facts(&self) -> String {
        #[cfg(windows)]
        let shell = if self.check("pwsh.exe", "exit 0").await == "runs" {
            "pwsh.exe"
        } else {
            "powershell.exe"
        };
        #[cfg(not(windows))]
        let shell = "/bin/sh";
        let (python, python3, node, npx) = tokio::join!(
            self.probe(shell, "python"),
            self.probe(shell, "python3"),
            self.probe(shell, "node"),
            self.probe(shell, "npx"),
        );
        let text = format!(
            "OS: {}; process_start shell: {shell}. Runtime checks for this app run: python={python}; python3={python3}; node={node}; npx={npx}.",
            std::env::consts::OS
        );
        #[cfg(windows)]
        let text = format!(
            "{text} npx.cmd={} (use this in PowerShell when npx.ps1 is blocked).",
            self.probe(shell, "npx.cmd").await
        );
        text
    }
}

/// Keeps machine facts out of the user's displayed message while sending them on every submission route.
pub(crate) async fn append(shared: &Shared, method: &str, params: &mut Value) {
    if !shared.app_context.enabled
        || !matches!(
            method,
            "turn/start" | "turn/steer" | "session/turn/enqueue" | "session/turn/queue/replace"
        )
    {
        return;
    }
    let Some(id) = params["sessionId"].as_str() else {
        return;
    };
    let facts = shared
        .app_context
        .facts
        .get_or_init(|| shared.app_context.facts())
        .await;
    let _ = shared.procmon.lease(id);
    let ports = shared.procmon.leases().of(id);
    let text = format!(
        "Vibe Work session facts. {facts}\nSession preview ports: {ports:?}. Call preview_list first for current canOpen/leasedFree ports; never assume a fixed port."
    );
    if method.starts_with("session/") {
        if let Some(entries) = params.get_mut("entries").and_then(Value::as_array_mut) {
            entries.insert(
                0,
                json!({"role":"context","content":[{"type":"text","text":text}]}),
            );
        }
    } else if let Some(message) = params.get_mut("message").and_then(Value::as_array_mut) {
        message.push(json!({"type":"resource","resource":{"kind":"text","uri":"vw://app/context","text":text}}));
    }
}

impl crate::Core {
    /// Validates input before saving attachment bytes or adding measured host context.
    pub(crate) async fn prepare_model_input(
        &self,
        method: &str,
        params: &mut Value,
        token: Option<&vw_gate::ConfirmationToken>,
    ) -> Result<(), crate::CoreError> {
        self.shared
            .gate
            .check(method, params, token)
            .map_err(|e| crate::CoreError::from_gate(method, &e))?;
        crate::attachment_save::prepare(&self.shared, method, params).await?;
        append(&self.shared, method, params).await;
        Ok(())
    }
}
