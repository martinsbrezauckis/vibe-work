//! Vault-owned proxy forwarding reuses netguard's HTTP/CONNECT, TLS and SOCKS transports and address policy.
//! Vibe, Python probes and core clients receive credential-free URLs. A lock cancels all forwarded connections.

use crate::{KeyError, KeyService, proxy_credentials::PROXIES};
use std::ffi::OsString;
use std::sync::Arc;
use vw_mistral::network::{self, NetworkSettings, has_sign_in};
use vw_netguard::{NetGuard, SystemResolver, Upstream};

pub(crate) fn bind(resolver: Arc<dyn vw_netguard::Resolve>) -> Result<NetGuard, KeyError> {
    let guard = NetGuard::bind(resolver)
        .map_err(|_| KeyError::Settings("The proxy forwarder could not start.".into()))?;
    guard.disable_canaries();
    guard.set_upstream(Upstream::none());
    guard.set_enabled(false);
    Ok(guard)
}

pub(crate) fn public(settings: &NetworkSettings, guard: &NetGuard) -> NetworkSettings {
    let mut result = settings.clone();
    for key in PROXIES {
        if settings.proxy_credentials.contains_key(key)
            || settings
                .get(key)
                .is_some_and(|v| has_sign_in(&v) || v.starts_with("socks"))
        {
            result = result.with(key, Some(guard.proxy_url()));
        }
    }
    result.proxy_credentials.clear();
    result.proxy_certificates.clear();
    result
}

/// Preserve credential-free routes at lock; an authenticated slot becomes an invalid route, never DIRECT.
pub(crate) fn locked(settings: &NetworkSettings) -> NetworkSettings {
    let mut clean = settings.clone();
    for key in PROXIES {
        if settings.proxy_credentials.contains_key(key)
            || settings.get(key).is_some_and(|v| has_sign_in(&v))
        {
            clean = clean.with(key, Some("vault-locked://proxy".into()));
        }
    }
    clean.proxy_credentials.clear();
    clean.proxy_certificates.clear();
    clean
}

pub(crate) fn configure_locked(guard: &NetGuard, clean: &NetworkSettings) {
    guard.set_enabled(false);
    guard.set_upstream(Upstream::none());
    if PROXIES.iter().any(|key| {
        clean
            .get(key)
            .is_some_and(|v| !v.starts_with("vault-locked:"))
    }) {
        configure(guard, clean);
    }
}

pub(crate) fn configure(guard: &NetGuard, settings: &NetworkSettings) {
    guard.set_upstream(
        Upstream::from_values(
            settings.http_proxy.as_deref(),
            settings.https_proxy.as_deref(),
            settings.all_proxy.as_deref(),
            settings.no_proxy.as_deref(),
        )
        .with_certificates(settings.guard_certificates())
        .with_proxy_certificates(settings.proxy_certificates.clone()),
    );
    guard.set_enabled(true);
}

impl KeyService {
    /// Credential-free settings for Vibe's file and helpers. Does not enable or change the running forwarder.
    #[must_use]
    pub fn child_network_settings(&self, settings: &NetworkSettings) -> NetworkSettings {
        public(settings, &self.vibe_forwarder)
    }

    /// Explicit child proxy values in both cases, overriding inherited sign-ins before spawning Vibe.
    #[must_use]
    pub fn proxy_env(&self) -> Vec<(String, OsString)> {
        let settings = NetworkSettings::from_environment();
        let public = public(&settings, &self.network_forwarder);
        network::KEYS
            .iter()
            .filter_map(|key| public.get(key).map(|v| (*key, v)))
            .flat_map(|(key, value)| {
                [
                    (key.to_owned(), OsString::from(&value)),
                    (key.to_ascii_lowercase(), OsString::from(value)),
                ]
            })
            .collect()
    }

    /// A temporary guard for the connection test; only Rust holds its upstream sign-in.
    /// # Errors
    /// The loopback listener could not bind.
    pub fn network_probe(
        &self,
        settings: &NetworkSettings,
        resolver: Option<Arc<dyn vw_netguard::Resolve>>,
    ) -> Result<(NetworkProbe, NetworkSettings), KeyError> {
        let _transition = self
            .network_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if crate::proxy_credentials::protected(settings) && !self.proxy_sign_in_ready() {
            return Err(KeyError::InvalidSecret(
                "Unlock the vault to use the proxy sign-in.",
            ));
        }
        let guard =
            vw_netguard::NetGuard::bind(resolver.unwrap_or_else(|| Arc::new(SystemResolver)))
                .map_err(|_| KeyError::Settings("The proxy forwarder could not start.".into()))?;
        guard.disable_canaries();
        configure(&guard, settings);
        guard.serve();
        let public = public(settings, &guard);
        self.network_probes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(guard.clone());
        Ok((
            NetworkProbe {
                guard,
                probes: self.network_probes.clone(),
            },
            public,
        ))
    }
}

impl KeyService {
    /// Register the existing browser guard. Its trusted settings never come from Vibe's writable .env.
    pub fn register_browser_guard(&self, guard: NetGuard) {
        self.browser_guards
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(guard);
        self.refresh_browser_guards();
    }

    pub(crate) fn refresh_browser_guards(&self) {
        let _transition = self
            .network_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let network = self
            .network
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let settings = &network.browser;
        for guard in self
            .browser_guards
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
        {
            if self.proxy_sign_in_ready() || !crate::proxy_credentials::protected(settings) {
                guard.set_upstream(
                    Upstream::from_values(
                        settings.http_proxy.as_deref(),
                        settings.https_proxy.as_deref(),
                        settings.all_proxy.as_deref(),
                        settings.no_proxy.as_deref(),
                    )
                    .with_system_resolver()
                    .with_certificates(settings.guard_certificates())
                    .with_proxy_certificates(settings.proxy_certificates.clone()),
                );
                guard.set_enabled(true);
            }
        }
    }
}

/// Owns the temporary Test forwarder. Cancellation drops its listeners and authenticated tunnels.
pub struct NetworkProbe {
    guard: NetGuard,
    probes: Arc<std::sync::Mutex<Vec<NetGuard>>>,
}
impl Drop for NetworkProbe {
    fn drop(&mut self) {
        self.guard.set_enabled(false);
        self.guard.shutdown();
        self.probes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|guard| guard.proxy_url() != self.guard.proxy_url());
    }
}
