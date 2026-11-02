//! Read-only session facts and connector suggestions for the agent. Uses core mode state and Vibe's native catalog.
//! Called by the session tool dispatcher; no path installs, enables or authenticates a connector.
use crate::{Core, Origin, core::Shared};
use serde_json::{Value, json};
use std::sync::Arc;
use vw_agent_tools::wire::tools;

pub(super) async fn call(
    shared: &Arc<Shared>,
    session: &str,
    tool: &str,
    args: &Value,
) -> Result<Value, String> {
    if shared.store.view(session).is_none() {
        return Err("this session is not open in Vibe Work right now".into());
    }
    let core = Core {
        shared: shared.clone(),
    };
    if tool == tools::SESSION_STATUS {
        let mode = shared.access.remembered(session);
        let label = mode
            .as_deref()
            .map(|mode| vw_vocabulary::label("accessModes", mode));
        return Ok(json!({
            "accessMode":mode,
            "accessModeName":label,
            "vibeVersion":core.status().vibe_version,
            "chromeExtensionStore":crate::chrome_control::commands::extension_store(shared),
        }));
    }
    // The catalog's transport future is large; keep the per-tool socket task's future bounded.
    let view = Box::pin(core.connectors_view(Origin::Ui, Some(session.to_owned()))).await;
    if let Some(error) = view.catalog_error {
        return Err(format!(
            "Mistral's connector catalog could not be read: {error}"
        ));
    }
    if tool == tools::CONNECTORS_LIST {
        return Ok(json!({"connectors":view.connectors}));
    }
    let alias = args
        .get("alias")
        .and_then(Value::as_str)
        .ok_or("alias is required")?;
    let reason = args
        .get("reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|reason| !reason.is_empty() && reason.chars().count() <= 500)
        .ok_or("reason must contain 1 to 500 characters")?;
    let connector = view
        .connectors
        .iter()
        .find(|connector| connector.alias == alias)
        .ok_or("This connector is not in Mistral's available catalog")?;
    if connector.readiness == "unavailable" {
        return Err("This connector is unavailable for this account".into());
    }
    Ok(
        json!({"connectorSuggestion":{"alias":connector.alias,"name":connector.display_name,"reason":reason}}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use vw_vibe_install::{ExtensionStore, InstallerConfig, Limits, Sources, VibeInstaller};

    #[tokio::test]
    async fn session_status_reports_the_recorded_extension_store_without_a_chrome_package() {
        for (index, store) in [
            Some(ExtensionStore::Review),
            Some(ExtensionStore::Available),
            None,
        ]
        .into_iter()
        .enumerate()
        {
            let core = crate::offline_core::start_with(
                &format!("session-extension-state-{index}"),
                |config, dir| {
                    let root = dir.join("install");
                    vw_vibe_install::extension_store::save(&root, store).unwrap();
                    config.vibe_install = Some(VibeInstaller::new(InstallerConfig {
                        root,
                        sources: Sources::pinned(&vw_vibe_install::this_target()),
                        parent_env: Vec::new(),
                        extra_env: Vec::new(),
                        discovery: vw_supervisor::Discovery {
                            explicit: None,
                            path_var: None,
                            home: None,
                        },
                        limits: Limits::default(),
                    }));
                },
            )
            .await;
            let _ = core.shared.store.load(json!({"session":{"id":"s","cwd":"/project","status":{"type":"idle"}},"entries":[],"turns":[]}), vw_session::LoadKind::New).unwrap();
            let status = call(&core.shared, "s", tools::SESSION_STATUS, &json!({}))
                .await
                .unwrap();
            core.shutdown().await;
            assert_eq!(
                status.get("chromeExtensionStore"),
                Some(&json!(store)),
                "{status}"
            );
            assert_eq!(
                status.get("vibeVersion"),
                Some(&Value::Null),
                "no running server is not a pinned version"
            );
        }
    }
}
