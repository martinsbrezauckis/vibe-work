//! Starts the vault and credential-free proxy routes before the supervisor. Called by Host startup.
//! Migrating a legacy proxy sign-in gates every Vibe launch until vault setup/unlock finishes.
use std::sync::Arc;
use tracing::{info, warn};
use vw_core_api::CoreConfig;
use vw_supervisor::{SecretProvider, SupervisorConfig};

/// Starts the key service when configured and adapts the launch to it (placeholder, overrides).
pub(super) async fn start_keys(
    keys: Option<vw_keys::KeyServiceConfig>,
    launch: &mut Result<SupervisorConfig, String>,
    secrets: &mut Arc<dyn SecretProvider>,
    core: &mut CoreConfig,
) -> Option<Arc<vw_keys::KeyService>> {
    // An unavailable key store never authorizes a custom network record.
    vw_mistral::network::set_current(vw_mistral::network::NetworkSettings::from_environment());
    let Some(config) = keys else {
        core.network_settings_rejected = core
            .network_file
            .as_deref()
            .is_some_and(|file| vw_mistral::network::load_for_launch(file, None).rejected);
        return None;
    };
    let supervisor = launch.as_mut().ok()?;
    match vw_keys::KeyService::start_with_network_home(
        config,
        core.network_file.clone(),
        supervisor.vibe_home.join(".env"),
    )
    .await
    {
        Ok(service) => {
            core.network_settings_rejected = service.network_settings_rejected();
            if let Some(list) = vw_mistral::network::current()
                .child_no_proxy(&vw_mistral::network::NetworkSettings::from_environment())
            {
                supervisor
                    .extra_env
                    .push(("NO_PROXY".to_owned(), list.into()));
            }
            supervisor.extra_env.extend(service.vibe_env());
            supervisor.extra_env.extend(service.proxy_env());
            let before = supervisor.before_launch.take();
            let network_keys = service.clone();
            supervisor.before_launch = Some(Arc::new(move |program| {
                network_keys.before_vibe_launch()?;
                before.as_ref().map_or(Ok(()), |check| check(program))
            }));
            *secrets = service.secret_provider();
            core.keys = Some(Arc::clone(&service));
            info!("key service started; Vibe gets a placeholder key");
            Some(service)
        }
        Err(error) => {
            warn!(%error, "the key service did not start");
            *launch = Err(format!("the key service did not start: {error}"));
            None
        }
    }
}
