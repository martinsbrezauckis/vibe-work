//! Command-started MCP servers follow Developer permissions: with it off none is started, with it on again the ones the
//! app paused are resumed.
//!
//! Where the state lives, and why: whether a server is *off* is Vibe's own `disabled` flag in `<own home>/config.toml`
//! (the only thing that stops Vibe starting it; the same flag the Tools page toggles). Who set it is not Vibe's to know:
//! the names the APP paused are kept in `settings.json` as `pausedCommandMcpServers`, next to `developerPermissions`.
//! A server the person turned off themselves is never in that list, so turning Developer permissions on never starts it.
//! `settings.json` is the app's own atomic store; Vibe rewrites `config.toml` and would drop a key it does not know.
//!
//! Depends on: `command_servers_config`, `app_settings`, `connectors` (refresh of open chats). Called by:
//! `developer_permissions` (the switch), `core_start` (before Vibe starts), `import_setup` (a brought server),
//! `connectors` (the Tools view, and the refusal to turn a paused server on).
//! Invariants:
//! - Pausing covers every `transport = "stdio"` entry whatever its origin: added in Tools, brought from another agent, or
//!   saved by an earlier version. Nothing is silent: the Tools view shows each paused server with the reason and the way back.
//! - Only the person's window turns Developer permissions on, so only then are servers resumed.
//! - Without an app data folder nothing can be recorded: servers are still paused, and none is ever resumed.

use std::path::Path;

use serde_json::{Map, Value};
use tracing::warn;

use crate::app_settings::{paused_command_servers, write_app_settings};
use crate::command_servers_config::update_disabled;
use crate::connectors_view::ServerView;
use crate::core::Shared;
use crate::developer_permissions::RefreshWithheldCause;
use crate::event::{AlertKind, CoreEvent};
use crate::mcp_keys_config::ConfigEditError;

/// Whether an enabled command row makes refresh unsafe while OFF. Also validates both persisted TOML
/// forms before resuming commands. The caller only uses this validation's error, not the blocked flag.
pub(crate) fn refresh_has_command_server(home: &Path) -> Result<bool, ConfigEditError> {
    crate::mcp_refresh::blocked_command(home, false, false)
}

/// Emits at most one visible Tools alert per settings revision and cause when forced
/// MCP server discovery is withheld.
pub(crate) fn notify_internal_refresh_withheld(
    shared: &Shared,
    session_id: Option<&str>,
    cause: RefreshWithheldCause,
) {
    let message = match cause {
        RefreshWithheldCause::DisabledCommandServer => {
            vw_vocabulary::texts::COMMAND_SERVER_DISABLED_REFRESH_BLOCKED
        }
        RefreshWithheldCause::PausedCommandServer => {
            warn!(
                session_id,
                "command servers: internal MCP refresh withheld while {} is off",
                vw_vocabulary::texts::DEVELOPER_PERMISSIONS_NAME
            );
            vw_vocabulary::texts::COMMAND_SERVER_INTERNAL_REFRESH_BLOCKED
        }
        RefreshWithheldCause::InspectFailed => {
            vw_vocabulary::texts::COMMAND_SERVER_INTERNAL_REFRESH_UNCHECKABLE
        }
    };
    if shared
        .developer_permissions
        .claim_refresh_withheld_notice(cause)
    {
        shared.emit(CoreEvent::Alert {
            session_id: session_id.map(str::to_owned),
            kind: AlertKind::ToolsUnavailable,
            message: message.to_owned(),
        });
    }
}

/// Admits an internal MCP server refresh under the Developer-permissions lock rule. Returns
/// `false` (after emitting the deduplicated Tools alert) when the refresh must remain withheld.
pub(crate) fn admit_internal_refresh(shared: &Shared, session_id: Option<&str>) -> bool {
    let enabled = shared.developer_permissions.enabled();
    match crate::mcp_refresh::blocked_command(
        &shared.instructions.own_home,
        enabled,
        crate::mcp_refresh::guard_installed(shared),
    ) {
        Ok(false) => true,
        Ok(true) => {
            notify_internal_refresh_withheld(
                shared,
                session_id,
                if enabled {
                    RefreshWithheldCause::DisabledCommandServer
                } else {
                    RefreshWithheldCause::PausedCommandServer
                },
            );
            false
        }
        Err(error) => {
            warn!(
                session_id,
                %error,
                "connectors: MCP refresh withheld while command servers cannot be checked"
            );
            notify_internal_refresh_withheld(
                shared,
                session_id,
                RefreshWithheldCause::InspectFailed,
            );
            false
        }
    }
}

const KEY: &str = "pausedCommandMcpServers";

fn record(settings: Option<&Path>, names: &[String]) {
    let Some(path) = settings else {
        return;
    };
    let value = if names.is_empty() {
        Value::Null
    } else {
        Value::Array(names.iter().cloned().map(Value::String).collect())
    };
    if let Err(error) = write_app_settings(path, Map::from_iter([(KEY.to_owned(), value)])) {
        warn!(%error, "command servers: the paused list was not saved");
    }
}

/// Pauses every enabled command server (Developer permissions is off) and records the names. Returns how many
/// were paused.
pub(crate) fn pause_all_checked(
    home: &Path,
    settings: Option<&Path>,
) -> Result<usize, ConfigEditError> {
    let changed = update_disabled(home, |_, disabled| (!disabled).then_some(true))?;
    if !changed.is_empty() {
        let mut names = settings.map(paused_command_servers).unwrap_or_default();
        for name in &changed {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
        record(settings, &names);
    }
    Ok(changed.len())
}

/// Records a newly imported command server already saved as disabled while the switch is OFF.
pub(crate) fn remember_paused(settings: Option<&Path>, name: &str) {
    let Some(path) = settings else {
        return;
    };
    let mut names = paused_command_servers(path);
    if !names.iter().any(|saved| saved == name) {
        names.push(name.to_owned());
        record(Some(path), &names);
    }
}

/// Resumes the command servers the app paused (and only those) and clears the list. Returns how many were resumed.
pub(crate) fn resume_paused(
    home: &Path,
    settings: Option<&Path>,
) -> Result<usize, ConfigEditError> {
    let Some(path) = settings else {
        return Ok(0);
    };
    let names = paused_command_servers(path);
    let resumed = update_disabled(home, |name, disabled| {
        (disabled && names.iter().any(|n| n == name)).then_some(false)
    })?;
    record(Some(path), &[]);
    Ok(resumed.len())
}

/// Forgets `name` in the paused list (it was removed, or the person turned it off or on themselves).
pub(crate) fn forget(settings: Option<&Path>, name: &str) {
    let Some(path) = settings else {
        return;
    };
    let target = crate::command_servers_config::vibe_server_name(name);
    if target.is_empty() {
        return;
    }
    let names = paused_command_servers(path);
    if names
        .iter()
        .any(|saved| crate::command_servers_config::vibe_server_name(saved) == target)
    {
        let rest: Vec<String> = names
            .into_iter()
            .filter(|saved| crate::command_servers_config::vibe_server_name(saved) != target)
            .collect();
        record(Some(path), &rest);
    }
}

/// Core start, before Vibe reads its config: with Developer permissions off, a command server (from an earlier
/// version, brought from another agent, or edited in by hand) is paused before Vibe can start it; with it on, the
/// servers the app paused earlier are resumed.
pub(crate) fn at_start(home: &Path, data: Option<&Path>) -> Result<(), ConfigEditError> {
    let settings = data.map(|d| d.join("settings.json"));
    let on = settings
        .as_deref()
        .is_some_and(crate::app_settings::developer_permissions_enabled);
    if on {
        // The app was closed (or the settings file edited) between turning the switch on and resuming.
        resume_paused(home, settings.as_deref()).map(|_| ())
    } else {
        pause_all_checked(home, settings.as_deref()).map(|_| ())
    }
}

/// Re-applies the pause when Developer permissions is off (a server turned on behind the app's back, or brought in a
/// moment ago). Returns true when something was paused.
pub(crate) fn enforce(shared: &Shared) -> Result<bool, ConfigEditError> {
    if shared.developer_permissions.enabled() {
        return Ok(false);
    }
    pause_all_checked(
        &shared.instructions.own_home,
        shared.developer_permissions.settings_path(),
    )
    .map(|count| count > 0)
}

/// Marks the paused command servers in the Tools view: off, with the reason and the way back.
pub(crate) fn project(shared: &Shared, servers: &mut [ServerView]) {
    if shared.developer_permissions.enabled() {
        return;
    }
    let path = shared.developer_permissions.settings_path();
    let paused = path.map(paused_command_servers).unwrap_or_default();
    for server in servers
        .iter_mut()
        .filter(|s| s.configured && s.transport == "stdio")
    {
        // A command server the person turned off keeps that state: only the ones the app paused are marked.
        if !paused.contains(&server.name) {
            continue;
        }
        server.off = true;
        server.error = Some(vw_vocabulary::texts::COMMAND_SERVER_PAUSED.to_owned());
        server.error_kind = Some("developerPermissionsOff".to_owned());
    }
}

#[cfg(test)]
mod normalized_name_tests {
    use super::{forget, remember_paused};

    #[test]
    fn a_persons_off_toggle_forgets_a_paused_name_through_its_vibe_alias() {
        let folder = tempfile::tempdir().unwrap();
        let settings = folder.path().join("settings.json");
        std::fs::write(&settings, "{}").unwrap();
        remember_paused(Some(&settings), "local-stdio");
        assert_eq!(
            crate::app_settings::paused_command_servers(&settings),
            ["local-stdio"]
        );
        forget(Some(&settings), "local-stdio-");
        assert_eq!(
            crate::app_settings::paused_command_servers(&settings),
            Vec::<String>::new()
        );
    }
}

#[cfg(test)]
mod inline_server_tests {
    use super::{at_start, pause_all_checked, resume_paused};
    use std::sync::Arc;

    // Pinned Vibe 3.1.4 Python: _write_toml_snapshot(path, RawConfig.model_validate(
    // {"active_model":"m","theme":"dark",
    // "mcp_servers":[{"name":"local","transport":"stdio","command":"/bin/echo","disabled":False},
    // {"name":"web","transport":"http","url":"https://x.invalid"}]})).
    const VIBE_INLINE: &str = "active_model = \"m\"\ntheme = \"dark\"\nmcp_servers = [\n    { name = \"local\", transport = \"stdio\", command = \"/bin/echo\", disabled = false },\n    { name = \"web\", transport = \"http\", url = \"https://x.invalid\" },\n]\n";
    // The same pinned writer with no `disabled` on local and a separate person-disabled stdio row.
    const VIBE_INLINE_UNSET: &str = "active_model = \"m\"\ntheme = \"dark\"\nmcp_servers = [\n    { name = \"local\", transport = \"stdio\", command = \"/bin/echo\" },\n    { name = \"person-off\", transport = \"stdio\", command = \"/bin/echo\", disabled = true },\n    { name = \"web\", transport = \"http\", url = \"https://x.invalid\" },\n]\n";

    #[test]
    fn off_pauses_vibe_inline_server_without_changing_other_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("vibe-home");
        std::fs::create_dir(&home).unwrap();
        std::fs::write(home.join("config.toml"), VIBE_INLINE).unwrap();
        let settings = dir.path().join("settings.json");
        assert_eq!(pause_all_checked(&home, Some(&settings)).unwrap(), 1);
        assert_eq!(
            std::fs::read_to_string(home.join("config.toml")).unwrap(),
            VIBE_INLINE.replace("disabled = false", "disabled = true")
        );
        assert_eq!(
            crate::app_settings::paused_command_servers(&settings),
            ["local"]
        );
    }

    #[test]
    fn at_start_pauses_vibe_inline_server() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("vibe-home");
        std::fs::create_dir(&home).unwrap();
        std::fs::write(home.join("config.toml"), VIBE_INLINE).unwrap();
        let _ = at_start(&home, Some(dir.path()));
        assert!(
            std::fs::read_to_string(home.join("config.toml"))
                .unwrap()
                .contains("disabled = true")
        );
        assert_eq!(
            crate::app_settings::paused_command_servers(&dir.path().join("settings.json")),
            ["local"]
        );
    }

    #[test]
    fn on_resumes_only_the_vibe_inline_server_the_app_paused() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("vibe-home");
        std::fs::create_dir(&home).unwrap();
        let paused = VIBE_INLINE.replace("disabled = false", "disabled = true");
        std::fs::write(home.join("config.toml"), &paused).unwrap();
        let settings = dir.path().join("settings.json");
        std::fs::write(&settings, r#"{"pausedCommandMcpServers":["local"]}"#).unwrap();
        assert_eq!(resume_paused(&home, Some(&settings)).unwrap(), 1);
        let after = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(!after.contains("disabled = true"), "{after}");
        assert!(
            after.contains("{ name = \"web\", transport = \"http\", url = \"https://x.invalid\" }")
        );
        assert_eq!(
            crate::app_settings::paused_command_servers(&settings),
            Vec::<String>::new()
        );
    }

    #[test]
    fn inline_unset_flag_is_added_and_person_disabled_row_stays_off() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("vibe-home");
        std::fs::create_dir(&home).unwrap();
        std::fs::write(home.join("config.toml"), VIBE_INLINE_UNSET).unwrap();
        let settings = dir.path().join("settings.json");
        assert_eq!(pause_all_checked(&home, Some(&settings)).unwrap(), 1);
        let expected = VIBE_INLINE_UNSET.replace(
            "{ name = \"local\", transport = \"stdio\", command = \"/bin/echo\" }",
            "{ name = \"local\", transport = \"stdio\", command = \"/bin/echo\", disabled = true }",
        );
        assert_eq!(
            std::fs::read_to_string(home.join("config.toml")).unwrap(),
            expected
        );
        assert_eq!(
            crate::app_settings::paused_command_servers(&settings),
            ["local"]
        );
        assert_eq!(resume_paused(&home, Some(&settings)).unwrap(), 1);
        assert_eq!(
            std::fs::read_to_string(home.join("config.toml")).unwrap(),
            VIBE_INLINE_UNSET
        );
    }

    #[test]
    fn unknown_server_shape_or_unclassifiable_row_refuses_pause() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("vibe-home");
        std::fs::create_dir(&home).unwrap();
        for config in [
            "mcp_servers = \"unknown\"\n",
            "mcp_servers = [42]\n",
            "mcp_servers = [{ name = \"local\", command = \"/bin/echo\" }]\n",
            "mcp_servers = [{ name = \"local\", transport = \"stdio\" }]\n",
        ] {
            std::fs::write(home.join("config.toml"), config).unwrap();
            assert!(pause_all_checked(&home, None).is_err(), "{config}");
        }
    }

    #[test]
    fn on_at_start_refuses_an_unclassifiable_saved_row() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("vibe-home");
        std::fs::create_dir(&home).unwrap();
        std::fs::write(home.join("config.toml"), "mcp_servers = [42]\n").unwrap();
        std::fs::write(
            dir.path().join("settings.json"),
            r#"{"developerPermissions":true,"pausedCommandMcpServers":["local"]}"#,
        )
        .unwrap();
        assert!(at_start(&home, Some(dir.path())).is_err());
    }

    #[tokio::test]
    async fn startup_refuses_an_uninspectable_server_list_before_launch() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("vibe-home");
        let data = dir.path().join("app-data");
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(&data).unwrap();
        std::fs::write(home.join("config.toml"), "mcp_servers = [42]\n").unwrap();
        let config = crate::CoreConfig {
            start_vibe: false,
            secret_scan_data: Some(data),
            ..crate::CoreConfig::default()
        };
        let result = Box::pin(crate::Core::start(
            vw_supervisor::SupervisorConfig::new(
                dir.path().join("no-vibe"),
                home,
                dir.path().to_path_buf(),
            ),
            Arc::new(vw_supervisor::NoSecret),
            DeclineConfirm,
            config,
        ))
        .await;
        match result {
            Ok(core) => {
                core.shutdown().await;
                panic!("startup accepted an uninspectable server list");
            }
            Err(error) => assert!(error.to_string().contains("command MCP servers"), "{error}"),
        }
    }

    #[tokio::test]
    async fn on_switch_leaves_permissions_off_if_server_rows_cannot_be_classified() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("vibe-home");
        let data = dir.path().join("app-data");
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(&data).unwrap();
        std::fs::write(home.join("config.toml"), "theme = 'dark'\n").unwrap();
        let config = crate::CoreConfig {
            start_vibe: false,
            secret_scan_data: Some(data),
            ..crate::CoreConfig::default()
        };
        let core = Box::pin(crate::Core::start(
            vw_supervisor::SupervisorConfig::new(
                dir.path().join("no-vibe"),
                home.clone(),
                dir.path().to_path_buf(),
            ),
            Arc::new(vw_supervisor::NoSecret),
            ApproveConfirm,
            config,
        ))
        .await
        .unwrap();
        std::fs::write(home.join("config.toml"), "mcp_servers = [42]\n").unwrap();
        let result = core
            .execute(
                crate::Origin::Ui,
                crate::Command::DeveloperPermissions {
                    enabled: Some(true),
                },
            )
            .await;
        assert!(result.is_err());
        assert!(!core.shared.developer_permissions.enabled());
        core.shutdown().await;
    }

    struct ApproveConfirm;

    impl vw_gate::Confirmer for ApproveConfirm {
        fn confirm(
            &self,
            _: &vw_gate::ConfirmationRequest,
        ) -> impl std::future::Future<Output = vw_gate::Confirmation> {
            std::future::ready(vw_gate::Confirmation::Approved)
        }
    }

    struct DeclineConfirm;

    impl vw_gate::Confirmer for DeclineConfirm {
        fn confirm(
            &self,
            _: &vw_gate::ConfirmationRequest,
        ) -> impl std::future::Future<Output = vw_gate::Confirmation> {
            std::future::ready(vw_gate::Confirmation::Declined)
        }
    }
}
