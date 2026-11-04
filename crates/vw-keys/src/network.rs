//! Network-record authentication through the same encrypted vault that holds the Mistral key.
//! The vault's item is the key. With "Lock the vault when Vibe Work quits" off, a copy in the computer's credential
//! store (the store that keeps the vault's own key) checks the record while the vault is locked, so a Lock does not
//! drop saved network settings; every Save writes a new key to both, and an open vault makes the copy equal to its
//! item. With the switch on no copy is kept (it is deleted when the switch is turned on, at quit and at start): a
//! locked start leaves the record waiting, and the unlock checks and applies it.
//! Called by host startup and core Settings after native confirmation; no method returns the MAC key.

use std::path::PathBuf;
use std::sync::PoisonError;

use vw_mistral::network::{self, LoadedSettings, NetworkSettings, PreparedSettings};
use vw_vault::{Vault, VaultStatus};

use crate::network_key::{check_key, rotate_mac_key, unlocked};
pub(crate) use crate::network_key::{drop_copy, switch_changed};
use crate::service::blocking;
use crate::{KeyError, KeyEvent, KeyService};
use crate::{
    proxy_credentials as credentials, proxy_forwarder as forwarder, proxy_migration as migration,
};

/// Why saved network settings wait for a vault unlock before they are checked and used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkWait {
    /// "Lock the vault when Vibe Work quits" is on: no copy of the check key is kept, so every start waits.
    LockAtQuit,
    /// This computer has no copy of the check key yet (an install from before the copy).
    NoCopyYet,
    /// A proxy sign-in is only available in an unlocked vault.
    ProxySignIn,
}

/// What the run did with the saved network record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Checked and used, or nothing custom was saved.
    Used,
    /// The record did not pass the check.
    Rejected,
    /// Not used yet: checked at the next vault unlock and, if it passes, used from then on.
    Waits(NetworkWait),
}

/// The run's network record: its file and what was done with it.
pub(crate) struct Network {
    state: State,
    file: Option<PathBuf>,
    pub(crate) dotenv: PathBuf,
    migration_pending: bool,
    pub(crate) requires_vault: bool,
    pub(crate) browser: NetworkSettings,
    core_locked: NetworkSettings,
    pub(crate) vibe_locked: NetworkSettings,
}

/// The start: checks and applies the record before any key-bearing client exists. With the switch on, any copy
/// is deleted first (a run that did not quit may have left one) and a locked vault leaves the record waiting.
pub(crate) fn launch(
    vault: &Vault,
    file: Option<PathBuf>,
    lock_on_quit: bool,
    dotenv: PathBuf,
    guard: &vw_netguard::NetGuard,
    vibe_guard: &vw_netguard::NetGuard,
) -> Result<Network, KeyError> {
    if lock_on_quit {
        drop_copy(vault);
    }
    let (key, locked) = check_key(vault, !lock_on_quit);
    let open = matches!(vault.status(), Ok(VaultStatus::Unlocked { .. }));
    let environment = NetworkSettings::from_environment();
    let mut loaded = file
        .as_deref()
        .map_or_else(LoadedSettings::default, |path| {
            network::read_settings(path, key.as_ref())
        });
    let legacy = file.as_deref().map_or(Ok(false), migration::legacy)?;
    let vibe = migration::read(&dotenv)?;
    let remnants = migration::remnant_sign_ins(file.as_deref(), &dotenv)?;
    let pending = legacy
        || !remnants.is_empty()
        || credentials::PROXIES
            .iter()
            .any(|key| vibe.get(key).is_some_and(|v| network::has_sign_in(&v)));
    if pending && !open {
        // No agent process may read legacy files or their remnants while migration waits for the vault.
        let core_locked = forwarder::locked(&environment);
        forwarder::configure_locked(guard, &core_locked);
        network::install_for_launch(
            &NetworkSettings::default(),
            &forwarder::public(&environment, guard),
        );
        return Ok(Network {
            state: State::Waits(NetworkWait::ProxySignIn),
            file,
            dotenv,
            migration_pending: true,
            requires_vault: true,
            browser: NetworkSettings::default(),
            core_locked,
            vibe_locked: NetworkSettings::default(),
        });
    }
    for settings in &remnants {
        credentials::store(vault, settings)?;
    }
    if legacy && let Some(path) = &file {
        migrate_core(vault, path, &mut loaded, !lock_on_quit)?;
    }
    let vibe = if open {
        migration::migrate(vault, &dotenv)?
    } else {
        vibe
    };
    // Vibe's writable file can supply routing only for Vibe. Core settings keep their MAC authority.
    let child = child_settings(&vibe, &loaded.settings);
    migration::write(&dotenv, &child, vibe_guard)?;
    if let Some(path) = &file {
        migration::cleanup(path)?;
    }
    let needs_unlock = credentials::protected(&child) || credentials::protected(&environment);
    let core_locked =
        forwarder::locked(&NetworkSettings::effective(&loaded.settings, &environment));
    let vibe_locked = forwarder::locked(&NetworkSettings::effective(&child, &environment));
    let browser = if open || !needs_unlock {
        let core = credentials::restore(vault, &loaded.settings)?;
        let vibe = credentials::restore(vault, &child)?;
        forwarder::configure(guard, &NetworkSettings::effective(&core, &environment));
        forwarder::configure(vibe_guard, &NetworkSettings::effective(&vibe, &environment));
        network::guard_settings(&core, &environment)
    } else {
        forwarder::configure_locked(guard, &core_locked);
        forwarder::configure_locked(vibe_guard, &vibe_locked);
        network::guard_settings(
            &forwarder::public(&loaded.settings, guard),
            &forwarder::public(&environment, guard),
        )
    };
    network::install_for_launch(
        &forwarder::public(&loaded.settings, guard),
        &forwarder::public(&environment, guard),
    );
    let state = if needs_unlock && !open {
        State::Waits(NetworkWait::ProxySignIn)
    } else if loaded.rejected && locked {
        State::Waits(if lock_on_quit {
            NetworkWait::LockAtQuit
        } else {
            NetworkWait::NoCopyYet
        })
    } else if loaded.rejected {
        State::Rejected
    } else {
        State::Used
    };
    Ok(Network {
        state,
        file,
        dotenv,
        migration_pending: false,
        requires_vault: needs_unlock,
        browser,
        core_locked,
        vibe_locked,
    })
}

impl KeyService {
    /// Whether a proxy sign-in may be used now. The MAC key's device-store copy grants no secret access.
    #[must_use]
    pub fn proxy_sign_in_ready(&self) -> bool {
        matches!(self.vault().status(), Ok(VaultStatus::Unlocked { .. }))
    }

    /// Persist a confirmed network candidate with vault-only sign-ins and a synced atomic replacement.
    /// # Errors
    /// The vault or settings file is unavailable.
    pub async fn persist_network_settings(
        &self,
        file: PathBuf,
        prepared: PreparedSettings,
    ) -> Result<(), KeyError> {
        let text = self.seal_network_settings(prepared).await?;
        blocking(move || migration::replace(&file, text.as_bytes())).await?
    }

    /// Reads the authenticated network record. Without the vault's key or its copy, custom values are rejected.
    pub async fn read_network_settings(&self, file: PathBuf) -> LoadedSettings {
        let (vault, copy) = (self.vault(), !self.lock_on_quit());
        blocking(move || {
            let (key, _) = check_key(&vault, copy);
            let mut loaded = network::read_settings(&file, key.as_ref());
            if !loaded.rejected {
                loaded.settings = credentials::restore(&vault, &loaded.settings)
                    .unwrap_or_else(|_| credentials::masked(&loaded.settings));
            }
            loaded
        })
        .await
        .unwrap_or_else(|_| LoadedSettings {
            settings: NetworkSettings::default(),
            rejected: true,
        })
    }

    /// Seals a confirmed candidate after storing a fresh hidden key, including for empty/default Saves.
    /// Older records then fail verification, even if writing the new record is interrupted.
    ///
    /// # Errors
    /// The vault is unavailable or the record is too large.
    pub async fn seal_network_settings(
        &self,
        prepared: PreparedSettings,
    ) -> Result<String, KeyError> {
        let (vault, copy) = (self.vault(), !self.lock_on_quit());
        blocking(move || {
            let public = credentials::store(&vault, prepared.settings())?;
            let prepared = network::prepare_settings(&public).map_err(|_| KeyError::Internal)?;
            let key = rotate_mac_key(&vault, copy)?;
            network::seal_settings(&prepared, Some(&key))
                .map_err(|e| KeyError::Settings(e.to_string()))
        })
        .await?
    }

    /// After an unlock: the copy follows the vault's key (switch off), and a record that waited for the vault is
    /// checked now. If it passes it is applied, and the key-bearing clients are built again before the key is read
    /// into the proxy (the caller refreshes the key after this).
    pub(crate) async fn network_unlocked(&self) {
        let (vault, copy) = (self.vault(), !self.lock_on_quit());
        let state = self.network.clone();
        let transition = self.network_use.clone();
        let guard = self.network_forwarder.clone();
        let vibe_guard = self.vibe_forwarder.clone();
        let checked = blocking(move || {
            let _transition = transition.lock().unwrap_or_else(PoisonError::into_inner);
            let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
            if copy {
                unlocked(&vault);
            }
            let network = launch(
                &vault,
                state.file.clone(),
                !copy,
                state.dotenv.clone(),
                &guard,
                &vibe_guard,
            )?;
            let rejected = network.state != State::Used;
            *state = network;
            Ok::<_, KeyError>(rejected)
        })
        .await;
        let rejected = checked.ok().and_then(Result::ok).unwrap_or(true);
        if !rejected {
            self.refresh_browser_guards();
            let proxy = self.proxy.lock().await;
            let built = proxy
                .as_ref()
                .map_or(Ok(()), |proxy| proxy.client().rebuild());
            if let Err(error) = built.and_then(|()| self.client.rebuild()) {
                tracing::warn!(%error, "network settings: a client was not built again");
            }
        }
        let _ = self
            .events
            .send(KeyEvent::NetworkSettingsChecked(!rejected));
    }

    pub(crate) fn stop_authenticated_network(&self) {
        let network = self.network.lock().unwrap_or_else(PoisonError::into_inner);
        if network.requires_vault {
            forwarder::configure_locked(&self.network_forwarder, &network.core_locked);
            forwarder::configure_locked(&self.vibe_forwarder, &network.vibe_locked);
        }
        for guard in self
            .network_probes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
        {
            guard.set_enabled(false);
            guard.set_upstream(vw_netguard::Upstream::none());
        }
    }

    pub(crate) fn network_locked(&self) {
        let mut network = self.network.lock().unwrap_or_else(PoisonError::into_inner);
        if credentials::protected(&network.browser) {
            for guard in self
                .browser_guards
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
            {
                forwarder::configure_locked(guard, &forwarder::locked(&network.browser));
            }
            network.browser = forwarder::locked(&network.browser);
        }
        if network.requires_vault {
            network.state = State::Waits(NetworkWait::ProxySignIn);
            let _ = self.events.send(KeyEvent::NetworkSettingsChecked(false));
        }
    }

    /// Refuses a Vibe spawn while a legacy file still carries a sign-in. Retrying after unlock is safe.
    /// # Errors
    /// Migration awaits an unlocked or newly created vault.
    pub fn before_vibe_launch(&self) -> Result<(), String> {
        if self
            .network
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .migration_pending
        {
            Err("Unlock or set up the vault to move the proxy sign-in, then restart Vibe.".into())
        } else {
            Ok(())
        }
    }

    /// Finish a confirmed Save: keep references in Vibe's file, sync both files and enable its upstream.
    /// # Errors
    /// The vault or the files are unavailable.
    pub async fn network_saved(
        &self,
        file: PathBuf,
        dotenv: PathBuf,
        changed: Vec<String>,
    ) -> Result<(), KeyError> {
        let (vault, copy) = (self.vault(), !self.lock_on_quit());
        self.network
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .dotenv
            .clone_from(&dotenv);
        let guard = self.network_forwarder.clone();
        let vibe_guard = self.vibe_forwarder.clone();
        let state = self.network.clone();
        let transition = self.network_use.clone();
        blocking(move || {
            let _transition = transition.lock().unwrap_or_else(PoisonError::into_inner);
            let (key, _) = check_key(&vault, copy);
            let loaded = network::read_settings(&file, key.as_ref());
            if loaded.rejected {
                return Err(KeyError::Internal);
            }
            let mut vibe = migration::read(&dotenv)?;
            for key in changed {
                vibe = vibe.with(&key, None);
            }
            let child = child_settings(&vibe, &loaded.settings);
            migration::write(&dotenv, &child, &vibe_guard)?;
            migration::cleanup(&file)?;
            let environment = NetworkSettings::from_environment();
            let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
            state.file = Some(file);
            state.requires_vault =
                credentials::protected(&child) || credentials::protected(&environment);
            state.core_locked =
                forwarder::locked(&NetworkSettings::effective(&loaded.settings, &environment));
            state.vibe_locked =
                forwarder::locked(&NetworkSettings::effective(&child, &environment));
            let vibe = credentials::restore(&vault, &child)?;
            let core = credentials::restore(&vault, &loaded.settings)?;
            forwarder::configure(&guard, &NetworkSettings::effective(&core, &environment));
            forwarder::configure(
                &vibe_guard,
                &NetworkSettings::effective(&vibe, &environment),
            );
            Ok(())
        })
        .await??;
        Ok(())
    }

    /// Whether the saved network record is not in use (rejected, or waiting for the vault); the core replays one
    /// notice.
    #[must_use]
    pub fn network_settings_rejected(&self) -> bool {
        self.network_state() != State::Used
    }

    /// Why the saved network record waits for a vault unlock, if it does.
    #[must_use]
    pub fn network_settings_wait(&self) -> Option<NetworkWait> {
        match self.network_state() {
            State::Waits(wait) => Some(wait),
            State::Used | State::Rejected => None,
        }
    }

    fn network_state(&self) -> State {
        self.network
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .state
    }
}

/// A writable Vibe CA path must never authenticate a vault password's upstream TLS peer.
/// Default roots come from verified core settings; credential-specific roots come from their vault items.
fn child_settings(vibe: &NetworkSettings, core: &NetworkSettings) -> NetworkSettings {
    let routing = vibe
        .clone()
        .with("SSL_CERT_FILE", None)
        .with("SSL_CERT_DIR", None);
    NetworkSettings::effective(&routing, core)
}

fn migrate_core(
    vault: &Vault,
    path: &std::path::Path,
    loaded: &mut LoadedSettings,
    copy: bool,
) -> Result<(), KeyError> {
    if loaded.rejected {
        // Quarantine the credential without granting an unverified file routing authority.
        credentials::store(vault, &migration::untrusted(path)?)?;
    } else {
        loaded.settings = credentials::store(vault, &loaded.settings)?;
    }
    let key = rotate_mac_key(vault, copy)?;
    let prepared = network::prepare_settings(&loaded.settings).map_err(|_| KeyError::Internal)?;
    let text = network::seal_settings(&prepared, Some(&key)).map_err(|_| KeyError::Internal)?;
    migration::replace(path, text.as_bytes())
}
