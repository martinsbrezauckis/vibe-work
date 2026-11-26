//! Vault-owned proxy forwarding reuses netguard's HTTP/CONNECT, TLS and SOCKS transports and address policy.
//! Vibe, Python probes and core clients receive per-run local credentials, never the upstream sign-in. A lock cancels all forwarded connections.

use crate::{KeyError, KeyService, proxy_credentials::PROXIES};
use std::ffi::OsString;
use std::sync::Arc;
use vw_mistral::network::{self, NetworkSettings, has_sign_in};
use vw_netguard::{NetGuard, SystemResolver, Upstream};
use vw_platform::sync::LockRecover;

pub(crate) fn bind(resolver: Arc<dyn vw_netguard::Resolve>) -> Result<NetGuard, KeyError> {
    let bound = if cfg!(any(windows, target_os = "macos")) {
        NetGuard::bind_system_client(resolver)
    } else {
        NetGuard::bind(resolver)
    };
    let guard =
        bound.map_err(|_| KeyError::Settings("The proxy forwarder could not start.".into()))?;
    guard.disable_canaries();
    guard.set_upstream(Upstream::none());
    guard.set_enabled(false);
    Ok(guard)
}

/// The core's key-bearing service owns an automatic route on every OS; Vibe's separate
/// agent-facing forwarder keeps its existing narrower Linux listener.
pub(crate) fn bind_service(resolver: Arc<dyn vw_netguard::Resolve>) -> Result<NetGuard, KeyError> {
    let guard = NetGuard::bind_system_client(resolver)
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
    if cfg!(any(windows, target_os = "macos"))
        || PROXIES.iter().any(|key| {
            clean
                .get(key)
                .is_some_and(|v| !v.starts_with("vault-locked:"))
        })
    {
        configure(guard, clean);
    }
}

pub(crate) fn configure(guard: &NetGuard, settings: &NetworkSettings) {
    let upstream = Upstream::from_values(
        settings.http_proxy.as_deref(),
        settings.https_proxy.as_deref(),
        settings.all_proxy.as_deref(),
        settings.no_proxy.as_deref(),
    );
    let upstream = if guard.is_system_client() {
        upstream.with_system_resolver()
    } else {
        upstream
    };
    guard.set_upstream(
        upstream
            .with_certificates(settings.guard_certificates())
            .with_proxy_certificates(settings.proxy_certificates.clone()),
    );
    guard.set_enabled(true);
}

impl KeyService {
    /// Locally authenticated settings for Vibe's private file and helpers. Does not enable or change the running forwarder.
    #[must_use]
    pub fn child_network_settings(&self, settings: &NetworkSettings) -> NetworkSettings {
        public(settings, &self.vibe_forwarder)
    }

    /// Explicit child proxy values in both cases, overriding inherited sign-ins before spawning Vibe.
    #[must_use]
    pub fn proxy_env(&self) -> Vec<(String, OsString)> {
        let settings = NetworkSettings::from_environment();
        let public = public(&settings, &self.network_forwarder);
        let values = network::KEYS
            .iter()
            .filter_map(|key| public.get(key).map(|v| (*key, v)))
            .flat_map(|(key, value)| {
                [
                    (key.to_owned(), OsString::from(&value)),
                    (key.to_ascii_lowercase(), OsString::from(value)),
                ]
            })
            .collect::<Vec<_>>();
        #[cfg(any(windows, target_os = "macos"))]
        let values = values
            .into_iter()
            .chain(automatic_env(
                &self.network.lock_or_recover().vibe_locked,
                &self.vibe_forwarder.proxy_url(),
            ))
            .collect();
        values
    }

    /// Exact owned capabilities selected by this launch's `proxy_env` snapshot, never upstream sign-ins.
    /// Linux keeps its existing environment credential policy and receives no exceptions.
    #[must_use]
    pub fn agent_proxy_relays(&self, env: &[(String, OsString)]) -> Vec<OsString> {
        proxy_relays(
            &self.network_forwarder,
            &self.vibe_forwarder,
            env,
            cfg!(any(windows, target_os = "macos")),
        )
    }

    /// A temporary guard for the connection test; only Rust holds its upstream sign-in.
    /// # Errors
    /// The loopback listener could not bind.
    pub fn network_probe(
        &self,
        settings: &NetworkSettings,
        resolver: Option<Arc<dyn vw_netguard::Resolve>>,
    ) -> Result<(NetworkProbe, NetworkSettings), KeyError> {
        let _transition = self.network_use.lock_or_recover();
        if crate::proxy_credentials::protected(settings) && !self.proxy_sign_in_ready() {
            return Err(KeyError::InvalidSecret(
                "Unlock the vault to use the proxy sign-in.",
            ));
        }
        let guard = bind(resolver.unwrap_or_else(|| Arc::new(SystemResolver)))?;
        configure(&guard, settings);
        guard.serve();
        let public = network::system_proxy_settings(&public(settings, &guard), &guard.proxy_url());
        self.network_probes.lock_or_recover().push(guard.clone());
        Ok((
            NetworkProbe {
                guard,
                probes: self.network_probes.clone(),
            },
            public,
        ))
    }
}

fn proxy_relays(
    network: &NetGuard,
    vibe: &NetGuard,
    env: &[(String, OsString)],
    native: bool,
) -> Vec<OsString> {
    if native {
        [network.proxy_url(), vibe.proxy_url()]
            .into_iter()
            .map(OsString::from)
            .filter(|url| {
                env.iter().any(|(name, value)| {
                    matches!(
                        name.to_ascii_uppercase().as_str(),
                        "HTTP_PROXY" | "HTTPS_PROXY" | "ALL_PROXY"
                    ) && value == url
                })
            })
            .collect()
    } else {
        Vec::new()
    }
}

impl KeyService {
    /// Register the existing browser guard. Its trusted settings never come from Vibe's writable .env.
    pub fn register_browser_guard(&self, guard: NetGuard) {
        self.browser_guards.lock_or_recover().push(guard);
        self.refresh_browser_guards();
    }

    pub(crate) fn refresh_browser_guards(&self) {
        let _transition = self.network_use.lock_or_recover();
        let network = self.network.lock_or_recover();
        let settings = &network.browser;
        for guard in self.browser_guards.lock_or_recover().iter() {
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
impl NetworkProbe {
    /// This temporary guard belongs to one connection test, so its refusal record identifies that test's lookup.
    #[must_use]
    pub fn proxy_lookup_failed(&self) -> bool {
        self.guard.proxy_lookup_failed_since(0)
    }

    /// Whether this test's selected upstream proxy could not be contacted.
    #[must_use]
    pub fn proxy_connect_failed(&self) -> bool {
        self.guard.proxy_connect_failed_since(0)
    }

    /// The Rust test retains its existing manual transport; only sign-ins and native automatic gaps use this relay.
    #[must_use]
    pub fn settings_for_rust_probe(&self, original: &NetworkSettings) -> NetworkSettings {
        let protected = PROXIES
            .iter()
            .any(|key| original.get(key).is_some_and(|value| has_sign_in(&value)));
        let settings = if protected {
            public(original, &self.guard)
        } else {
            original.clone()
        };
        network::system_proxy_settings(&settings, &self.guard.proxy_url())
    }

    /// The selected test route, without endpoints or credentials. A failed forwarded attempt has no route.
    /// Unforwarded clients retain their existing transport and use the same scheme/bypass matching as reqwest.
    #[must_use]
    pub fn route_used(
        &self,
        original: &NetworkSettings,
        child: &NetworkSettings,
        target: &str,
    ) -> Option<bool> {
        let url = url::Url::parse(target).ok()?;
        let matches = |settings: &NetworkSettings| {
            let bypass = network::with_loopback(settings.no_proxy.as_deref().unwrap_or_default());
            Upstream::from_values(
                settings.http_proxy.as_deref(),
                settings.https_proxy.as_deref(),
                settings.all_proxy.as_deref(),
                Some(&bypass),
            )
            .configured_proxy_for(&url)
            .ok()
        };
        let scheme = if url.scheme() == "https" {
            &child.https_proxy
        } else {
            &child.http_proxy
        };
        let forwarded = scheme
            .as_ref()
            .or(child.all_proxy.as_ref())
            .is_some_and(|endpoint| endpoint == &self.guard.proxy_url())
            && matches(child)?;
        if forwarded {
            self.guard
                .last_route()
                .map(|route| route == vw_netguard::upstream::UsedRoute::Proxy)
        } else {
            matches(original)
        }
    }
}
impl Drop for NetworkProbe {
    fn drop(&mut self) {
        self.guard.set_enabled(false);
        self.guard.shutdown();
        self.probes
            .lock_or_recover()
            .retain(|guard| guard.proxy_url() != self.guard.proxy_url());
    }
}

#[cfg(any(windows, target_os = "macos", test))]
fn automatic_env(settings: &NetworkSettings, endpoint: &str) -> Vec<(String, OsString)> {
    if settings.all_proxy.is_some() {
        return Vec::new();
    }
    let mut values = Vec::new();
    for key in ["HTTP_PROXY", "HTTPS_PROXY"] {
        if settings.get(key).is_none() {
            values.push((key.to_owned(), OsString::from(endpoint)));
            values.push((key.to_ascii_lowercase(), OsString::from(endpoint)));
        }
    }
    if !values.is_empty() {
        let bypass = network::with_loopback(settings.no_proxy.as_deref().unwrap_or_default());
        values.push(("NO_PROXY".into(), OsString::from(&bypass)));
        values.push(("no_proxy".into(), OsString::from(bypass)));
    }
    values
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    const RELAY: &str = "http://local:capability@127.0.0.1:19000";

    #[test]
    fn child_proxy_allowance_contains_only_current_owned_relays_on_native_platforms() {
        let network = NetGuard::bind(Arc::new(SystemResolver)).unwrap();
        let vibe = NetGuard::bind(Arc::new(SystemResolver)).unwrap();
        let stale = NetGuard::bind(Arc::new(SystemResolver)).unwrap();
        let env = vec![
            ("HTTP_PROXY".into(), network.proxy_url().into()),
            ("HTTPS_PROXY".into(), vibe.proxy_url().into()),
            ("ALL_PROXY".into(), stale.proxy_url().into()),
        ];
        let allowed = proxy_relays(&network, &vibe, &env, true);
        assert_eq!(allowed.len(), 2);
        assert!(allowed.contains(&OsString::from(network.proxy_url())));
        assert!(allowed.contains(&OsString::from(vibe.proxy_url())));
        assert!(!allowed.contains(&OsString::from(stale.proxy_url())));
        assert_eq!(
            proxy_relays(&network, &vibe, &env, false),
            Vec::<OsString>::new()
        );
    }

    #[test]
    fn owned_but_unselected_relay_is_not_allowed_for_inherited_proxy_values() {
        let network = NetGuard::bind(Arc::new(SystemResolver)).unwrap();
        let vibe = NetGuard::bind(Arc::new(SystemResolver)).unwrap();
        let env = vec![("HTTPS_PROXY".into(), vibe.proxy_url().into())];
        let allowed = proxy_relays(&network, &vibe, &env, true);
        assert_eq!(allowed.len(), 1);
        assert!(allowed.contains(&OsString::from(vibe.proxy_url())));
        assert!(!allowed.contains(&OsString::from(network.proxy_url())));
        assert_eq!(
            proxy_relays(&network, &vibe, &[], true),
            Vec::<OsString>::new()
        );
        let misplaced = vec![("NO_PROXY".into(), vibe.proxy_url().into())];
        assert_eq!(
            proxy_relays(&network, &vibe, &misplaced, true),
            Vec::<OsString>::new()
        );
    }

    #[test]
    fn vibe_automatic_environment_fills_only_missing_schemes_in_both_cases() {
        let settings = NetworkSettings::default()
            .with("HTTPS_PROXY", Some("http://manual.test:8080".into()))
            .with("NO_PROXY", Some("inside.test".into()));
        let values: std::collections::BTreeMap<_, _> =
            automatic_env(&settings, RELAY).into_iter().collect();
        assert_eq!(values.get("HTTP_PROXY"), Some(&OsString::from(RELAY)));
        assert_eq!(values.get("http_proxy"), Some(&OsString::from(RELAY)));
        assert!(!values.contains_key("HTTPS_PROXY"));
        assert!(!values.contains_key("https_proxy"));
        for key in ["NO_PROXY", "no_proxy"] {
            let no = values.get(key).unwrap().to_str().unwrap();
            assert!(
                no.contains("127.0.0.1")
                    && no.contains("localhost")
                    && no.contains("::1")
                    && no.contains("inside.test")
            );
        }
    }

    #[test]
    fn vibe_all_proxy_and_complete_manual_routes_are_not_replaced() {
        for settings in [
            NetworkSettings::default().with("ALL_PROXY", Some("http://manual.test:8080".into())),
            NetworkSettings::default()
                .with("HTTP_PROXY", Some("http://manual.test:8080".into()))
                .with("HTTPS_PROXY", Some("http://manual.test:8081".into())),
        ] {
            assert_eq!(automatic_env(&settings, RELAY), Vec::new());
        }
    }

    #[test]
    fn rust_test_preserves_the_existing_manual_socks_transport() {
        let guard = NetGuard::bind(Arc::new(SystemResolver)).unwrap();
        let probe = NetworkProbe {
            guard,
            probes: Arc::default(),
        };
        let settings =
            NetworkSettings::default().with("ALL_PROXY", Some("socks5h://manual.test:1080".into()));
        assert_eq!(probe.settings_for_rust_probe(&settings), settings);
    }
}
