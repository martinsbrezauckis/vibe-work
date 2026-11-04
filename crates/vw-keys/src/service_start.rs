//! Constructs the key service after vault restoration and proxy migration. Called by host startup and fixtures.
//! No client or Vibe child receives an unverified network record or a proxy sign-in.
use super::{
    Arc, KeyCell, KeyError, KeyEvent, KeyNeed, KeyService, KeyServiceConfig, KeySettings, KeyState,
    PathBuf, PoisonError, ProxyHandle, RateLimits, TokenSlot, Vault, VaultSettings, blocking,
    broadcast, last_state,
};

impl KeyService {
    /// Starts after verifying saved network settings with the existing vault, before any key-bearing client exists.
    ///
    /// # Errors
    /// As [`Self::start`]. An unavailable vault leaves custom network settings unapplied.
    pub async fn start_with_network(
        config: KeyServiceConfig,
        network_file: Option<PathBuf>,
    ) -> Result<Arc<Self>, KeyError> {
        let dotenv = config.existing.dotenv.clone();
        Self::start_with_network_home(config, network_file, dotenv).await
    }

    /// Starts with the app-owned Vibe home's environment file for credential migration.
    /// # Errors
    /// As [`Self::start_with_network`]; migration failures never permit Vibe to load plaintext credentials.
    pub async fn start_with_network_home(
        config: KeyServiceConfig,
        network_file: Option<PathBuf>,
        dotenv: PathBuf,
    ) -> Result<Arc<Self>, KeyError> {
        Self::start_with_network_resolver(
            config,
            network_file,
            dotenv,
            Arc::new(vw_netguard::SystemResolver),
        )
        .await
    }

    /// Starts with a destination resolver, allowing isolated proxy transport tests without external DNS.
    /// # Errors
    /// As [`Self::start_with_network_home`].
    pub async fn start_with_network_resolver(
        config: KeyServiceConfig,
        network_file: Option<PathBuf>,
        dotenv: PathBuf,
        resolver: Arc<dyn vw_netguard::Resolve>,
    ) -> Result<Arc<Self>, KeyError> {
        let vault = Arc::new(match config.managed_vault {
            Some(data_dir) => Vault::new_managed(config.vault, data_dir)?,
            None => Vault::new(config.vault)?,
        });
        if config.settings_path.is_file() {
            vault.select_legacy_if_used()?;
        }
        let vault_settings = VaultSettings::next_to(&config.settings_path);
        let (network_vault, unlock_vault) = (Arc::clone(&vault), Arc::clone(&vault));
        let (at_start, vault_settings) = blocking(move || {
            let at_start = last_state::restore(&unlock_vault, &vault_settings);
            (at_start, vault_settings)
        })
        .await?;
        let lock_on_quit = at_start.lock_on_quit;
        let network_forwarder = crate::proxy_forwarder::bind(resolver.clone())?;
        let vibe_forwarder = crate::proxy_forwarder::bind(resolver)?;
        let forwarder = network_forwarder.clone();
        let vibe_guard = vibe_forwarder.clone();
        let network = blocking(move || {
            crate::network::launch(
                &network_vault,
                network_file,
                lock_on_quit,
                dotenv,
                &forwarder,
                &vibe_guard,
            )
        })
        .await??;
        let (events, _) = broadcast::channel(64);
        let tokens = Arc::new(TokenSlot::default());
        let key = KeyCell::new(KeyState::Missing(KeyNeed::Unlock));
        let hook_events = events.clone();
        let mut proxy_config = config.proxy;
        let mcp = proxy_config.mcp.clone();
        proxy_config.on_wait = Arc::new(move |need| {
            let _ = hook_events.send(KeyEvent::KeyNeeded(need));
        });
        let limits: Arc<std::sync::Mutex<Option<RateLimits>>> = Arc::default();
        let (limits_cell, limits_events) = (Arc::clone(&limits), events.clone());
        proxy_config.on_rate_limits = Arc::new(move |snapshot| {
            *limits_cell.lock().unwrap_or_else(PoisonError::into_inner) = Some(snapshot);
            let _ = limits_events.send(KeyEvent::RateLimits(Some(snapshot)));
        });
        let proxy = ProxyHandle::start(proxy_config, Arc::clone(&tokens), key.clone())
            .await
            .map_err(|e| KeyError::Settings(e.to_string()))?;
        // No redirects: the key goes only to the host we chose (test call and OCR send it directly).
        let client =
            vw_mistral::ClientSlot::new().map_err(|e| KeyError::Settings(e.to_string()))?;
        let service = Arc::new(Self {
            vault,
            proxy_base: proxy.base_url(),
            proxy: tokio::sync::Mutex::new(Some(proxy)),
            key,
            tokens,
            client,
            test_api_base: config.test_api_base,
            last_key_test: std::sync::Mutex::default(),
            chrome_owner_account: crate::chrome::account(&config.settings_path),
            settings: KeySettings::new(config.settings_path),
            vault_settings,
            lock_on_quit: std::sync::atomic::AtomicBool::new(at_start.lock_on_quit),
            locked_until_password: std::sync::atomic::AtomicBool::new(
                at_start.locked_until_password,
            ),
            selection: tokio::sync::Mutex::new(()),
            existing: config.existing,
            events,
            cleanup_notices: Arc::default(),
            limits,
            mcp,
            login_counters: crate::logins::Counters::default(),
            network: Arc::new(std::sync::Mutex::new(network)),
            network_use: Arc::default(),
            network_probes: Arc::default(),
            network_forwarder,
            vibe_forwarder,
            browser_guards: std::sync::Mutex::default(),
        });
        service.network_forwarder.serve();
        service.vibe_forwarder.serve();
        service.refresh_key().await;
        Ok(service)
    }
}
