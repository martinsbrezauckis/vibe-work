//! Secret commands reuse the server's PATH and, on Windows, its actual Git Bash launcher PATH.
//! Depends on the supervisor environment builder and pinned Vibe's shell resolver. Called before
//! pinning a secret command; the probe receives only the process allowlist, never vault values.

#[cfg(windows)]
use std::ffi::OsString;

/// Keep explicit server PATH settings in the ordinary process allowlist too.
pub(crate) fn inherit_path(
    server: &vw_supervisor::SupervisorConfig,
    config: &mut crate::CoreConfig,
) -> Result<(), vw_supervisor::SupervisorError> {
    let Some(tools) = config.agent_tools.as_mut() else {
        return Ok(());
    };
    let env = vw_supervisor::build_env(&vw_supervisor::EnvPlan {
        parent: server
            .parent_env
            .clone()
            .unwrap_or_else(|| std::env::vars_os().collect()),
        extra: &server.extra_env,
        vibe_home: &server.vibe_home,
        home: server.home.as_deref(),
        key: None,
        mcp_token: None,
    })?;
    if let Some((_, path)) = env
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("PATH"))
    {
        tools
            .process_env
            .retain(|(name, _)| !name.eq_ignore_ascii_case("PATH"));
        tools.process_env.push(("PATH".into(), path.clone()));
    }
    Ok(())
}

#[cfg(windows)]
pub(super) async fn for_run(
    shared: &crate::core::Shared,
    tools: &super::AgentTools,
    cwd: &std::path::Path,
) -> Result<Vec<(OsString, OsString)>, String> {
    let python = vw_supervisor::server_python(&shared.supervisor.program())
        .ok_or("Vibe's Python could not be found to read the agent shell PATH")?;
    let mut command = tokio::process::Command::new(python);
    // The launcher can add its own tool folders even when usr/bin/bash.exe, named in the
    // secret request, does not. Ask the same resolver/launcher as Vibe; do not guess folders.
    command
        .args(["-I", "-B", "-c", include_str!("secret_shell_path.py")])
        .current_dir(cwd)
        .env_clear()
        .envs(tools.config.process_env.clone())
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .creation_flags(0x0800_0000);
    let output = tokio::time::timeout(std::time::Duration::from_secs(10), command.output())
        .await
        .map_err(|_| "The agent shell PATH probe timed out")?
        .map_err(|_| "The agent shell PATH probe could not start")?;
    if !output.status.success() {
        return Err("The agent shell PATH could not be read".into());
    }
    let path: String = serde_json::from_slice(&output.stdout)
        .map_err(|_| "The agent shell PATH was not valid text")?;
    let mut env = tools.config.process_env.clone();
    env.retain(|(name, _)| !name.eq_ignore_ascii_case("PATH"));
    env.push(("PATH".into(), path.into()));
    Ok(env)
}
