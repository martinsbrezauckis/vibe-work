//! Confirmed raw proxy writes for Vibe's own routing. Uses the vault and existing Vibe forwarder; never changes core routing.
//! Each stored credential binds its TLS roots so subsequent edits to Vibe's CA paths cannot reveal its password.
use crate::{
    KeyError, KeyService, proxy_credentials as credentials, proxy_forwarder as forwarder,
    proxy_migration as migration, service::blocking,
};
use std::{collections::BTreeMap, path::PathBuf, sync::PoisonError};
use vw_mistral::network::NetworkSettings;

impl KeyService {
    /// Pause a route before a confirmed write publishes its credential-free address to Vibe.
    pub fn pause_vibe_proxy(&self, settings: &NetworkSettings) {
        if credentials::protected(settings) {
            let _transition = self
                .network_use
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            self.network
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .requires_vault = true;
            self.vibe_forwarder.set_enabled(false);
            self.vibe_forwarder
                .set_upstream(vw_netguard::Upstream::none());
        }
    }

    /// Store an already confirmed raw Vibe write without granting it authority over the core's clients.
    /// # Errors
    /// The vault or Vibe's file is unavailable.
    pub async fn prepare_vibe_proxy_write(
        &self,
        dotenv: PathBuf,
        changes: BTreeMap<String, Option<String>>,
    ) -> Result<NetworkSettings, KeyError> {
        let vault = self.vault();
        let transition = self.network_use.clone();
        let settings = blocking(move || {
            let _transition = transition.lock().unwrap_or_else(PoisonError::into_inner);
            let mut settings = migration::read(&dotenv)?;
            for (key, value) in changes {
                settings.proxy_credentials.remove(&key);
                settings.proxy_certificates.remove(&key);
                settings = settings.with(&key, value);
            }
            credentials::store(&vault, &settings)
        })
        .await??;
        self.pause_vibe_proxy(&settings);
        Ok(settings)
    }

    /// Complete the gated write, keeping only references in Vibe's file and applying its own upstream.
    /// # Errors
    /// The vault is locked or a durable file replacement failed.
    pub async fn finish_vibe_proxy_write(
        &self,
        dotenv: PathBuf,
        settings: NetworkSettings,
    ) -> Result<(), KeyError> {
        let vault = self.vault();
        let transition = self.network_use.clone();
        let guard = self.vibe_forwarder.clone();
        let state = self.network.clone();
        blocking(move || {
            let _transition = transition.lock().unwrap_or_else(PoisonError::into_inner);
            let restored = credentials::restore(&vault, &settings)?;
            migration::write(&dotenv, &settings, &guard)?;
            let environment = NetworkSettings::from_environment();
            // Credential-specific roots from the vault override the writable Vibe CA paths.
            forwarder::configure(&guard, &NetworkSettings::effective(&restored, &environment));
            let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
            state.dotenv = dotenv;
            state.vibe_locked =
                forwarder::locked(&NetworkSettings::effective(&settings, &environment));
            state.requires_vault |=
                credentials::protected(&settings) || credentials::protected(&environment);
            Ok(())
        })
        .await?
    }
}
