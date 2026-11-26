//! Runtime-only automatic proxy relay for core clients. The key service supplies its owned listener;
//! saved settings and Vibe's writable files never supply this endpoint. Linux retains its existing routing.
use super::{NetworkSettings, with_loopback};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

static RELAY: RwLock<Option<Arc<Relay>>> = RwLock::new(None);
const NATIVE: bool = cfg!(any(windows, target_os = "macos"));

struct Relay {
    endpoint: String,
    active: AtomicBool,
}

/// The key service's runtime relay registration. Dropping it revokes retained clients too.
pub struct SystemProxy(Arc<Relay>);

impl SystemProxy {
    /// Revoke this owner's clients and remove its endpoint if it is still current.
    pub fn clear(&self) {
        self.0.active.store(false, Ordering::SeqCst);
        clear_system_proxy_if(&self.0.endpoint);
    }
}

impl Drop for SystemProxy {
    fn drop(&mut self) {
        self.clear();
    }
}

/// Install the app-owned automatic proxy relay before constructing core clients.
/// Keep the returned owner until shutdown; an early startup error revokes it automatically.
#[must_use]
pub fn set_system_proxy(endpoint: String) -> SystemProxy {
    let relay = Arc::new(Relay {
        endpoint,
        active: AtomicBool::new(true),
    });
    *RELAY.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(&relay));
    SystemProxy(relay)
}

/// Whether an update's HTTPS route is backed by verified launch settings and either a usable
/// manual proxy or the key service's live automatic relay.
#[must_use]
pub fn update_route_ready() -> bool {
    let verified = super::ROUTE_VERIFIED.load(Ordering::SeqCst);
    let settings = super::current();
    let relay = RELAY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .is_some_and(|relay| relay.active.load(Ordering::SeqCst));
    route_ready_for(&settings, verified, relay)
}

fn route_ready_for(settings: &NetworkSettings, verified: bool, relay: bool) -> bool {
    if !verified {
        return false;
    }
    if settings.https_proxy.is_some() || settings.all_proxy.is_some() {
        return manual_route_for(settings, "https");
    }
    relay
}

fn manual_route_for(settings: &NetworkSettings, scheme: &str) -> bool {
    let selected = match scheme {
        "http" => settings
            .http_proxy
            .as_ref()
            .and_then(|url| reqwest::Proxy::http(url).ok()),
        "https" => settings
            .https_proxy
            .as_ref()
            .and_then(|url| reqwest::Proxy::https(url).ok()),
        _ => None,
    };
    selected.is_some()
        || settings
            .all_proxy
            .as_ref()
            .is_some_and(|url| reqwest::Proxy::all(url).is_ok())
}

/// Remove only the matching owner's endpoint, returning whether it was current.
pub fn clear_system_proxy_if(endpoint: &str) -> bool {
    let mut current = RELAY.write().unwrap_or_else(PoisonError::into_inner);
    if current
        .as_ref()
        .is_some_and(|relay| relay.endpoint == endpoint)
    {
        if let Some(relay) = current.take() {
            relay.active.store(false, Ordering::SeqCst);
        }
        true
    } else {
        false
    }
}

/// Fill only native automatic HTTP(S) routes, retaining manual endpoints and the local model bypass.
/// The result is runtime-only: never persist its local relay capability in saved settings.
#[must_use]
pub fn system_proxy_settings(settings: &NetworkSettings, endpoint: &str) -> NetworkSettings {
    project(settings, endpoint, NATIVE)
}

pub(super) fn apply(
    settings: &NetworkSettings,
    builder: reqwest::ClientBuilder,
) -> reqwest::ClientBuilder {
    let relay = RELAY.read().unwrap_or_else(PoisonError::into_inner).clone();
    apply_relay(settings, builder, relay, NATIVE)
}

fn apply_relay(
    settings: &NetworkSettings,
    builder: reqwest::ClientBuilder,
    relay: Option<Arc<Relay>>,
    native: bool,
) -> reqwest::ClientBuilder {
    if !native {
        return settings.apply_values(builder);
    }
    let Some(relay) = relay.filter(|_| {
        settings.all_proxy.is_none()
            && (settings.http_proxy.is_none() || settings.https_proxy.is_none())
    }) else {
        // Suppress reqwest's implicit system proxy here. Loopback is direct, configured manual
        // request types use their own proxy, and an automatic route needs the owned relay.
        let builder = settings.apply_values(builder.no_proxy());
        let settings = settings.clone();
        return builder.connector_layer(tower::filter::FilterLayer::new(move |request| {
            if direct_or_manual(&settings, &request) {
                Ok(request)
            } else {
                Err(std::io::Error::other("the system proxy route is not ready"))
            }
        }));
    };
    // Check before opening any new socket, including to a process that reused the old port.
    // The service also disables its guard to close existing pooled tunnels at teardown.
    // Each request opens a new relay connection and gets a fresh native route. HTTP/1 plus no
    // idle pool also keeps a concurrent request off an active HTTP/2 connection with an old route.
    let builder = builder.pool_max_idle_per_host(0).http1_only();
    let manual = settings.clone();
    project(settings, &relay.endpoint, native)
        .apply_values(builder)
        .connector_layer(tower::filter::FilterLayer::new(move |request| {
            if relay.active.load(Ordering::SeqCst) || direct_or_manual(&manual, &request) {
                Ok(request)
            } else {
                Err(std::io::Error::other("the system proxy owner has stopped"))
            }
        }))
}

/// Reqwest's connector layer hides its URI in a Debug-only wrapper. Accept only the known
/// wrapper and a parsed HTTP(S) destination; an unknown representation stays closed. This check
/// runs before reqwest selects a proxy, so manual routes are accepted by request scheme.
fn direct_or_manual(settings: &NetworkSettings, request: &impl std::fmt::Debug) -> bool {
    let shown = format!("{request:?}");
    let Some(uri) = shown
        .strip_prefix("Unnameable(")
        .and_then(|text| text.strip_suffix(')'))
    else {
        return false;
    };
    let Ok(url) = reqwest::Url::parse(uri) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost") || matches!(host, "127.0.0.1" | "[::1]")
    });
    loopback || manual_route_for(settings, url.scheme())
}

fn project(settings: &NetworkSettings, endpoint: &str, native: bool) -> NetworkSettings {
    let mut projected = settings.clone();
    if !native || settings.all_proxy.is_some() {
        return projected;
    }
    let mut filled = false;
    for key in ["HTTP_PROXY", "HTTPS_PROXY"] {
        if settings.get(key).is_none() {
            projected = projected.with(key, Some(endpoint.to_owned()));
            filled = true;
        }
    }
    if filled {
        projected.no_proxy = Some(with_loopback(
            settings.no_proxy.as_deref().unwrap_or_default(),
        ));
    }
    projected
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    const RELAY: &str = "http://local:capability@127.0.0.1:19000";
    static GLOBAL_ROUTE_TEST: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[tokio::test]
    async fn retained_client_cannot_send_to_a_rebound_relay_port() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let relay = Arc::new(super::Relay {
            endpoint: format!("http://{address}"),
            active: AtomicBool::new(true),
        });
        let owner = SystemProxy(Arc::clone(&relay));
        let client = apply_relay(
            &NetworkSettings::default(),
            crate::tls_builder().no_proxy(),
            Some(relay),
            true,
        )
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();
        let first = tokio::spawn(receipt(listener, "LIVE"));
        assert_eq!(
            client
                .get("http://relay-lifetime.test/")
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "LIVE"
        );
        first.await.unwrap();
        owner.clear();
        let rebound = tokio::net::TcpListener::bind(address).await.unwrap();
        assert!(
            client
                .get("http://relay-lifetime.test/")
                .send()
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), rebound.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn reviewer_owner_end_clears_global_endpoint_and_old_owner_preserves_new() {
        let _serial = GLOBAL_ROUTE_TEST.lock().await;
        let old = set_system_proxy(RELAY.into());
        let snapshot = super::RELAY.read().unwrap().clone().unwrap();
        assert_eq!(
            project(&NetworkSettings::default(), &snapshot.endpoint, true)
                .https_proxy
                .as_deref(),
            Some(RELAY)
        );
        old.clear();
        assert!(
            super::RELAY.read().unwrap().is_none(),
            "a new key-bearing client still gets the old listener"
        );
        let old = set_system_proxy(RELAY.into());
        let new_endpoint = "http://new:capability@127.0.0.1:19001";
        let new = set_system_proxy(new_endpoint.into());
        drop(old);
        assert_eq!(
            super::RELAY.read().unwrap().as_ref().unwrap().endpoint,
            new_endpoint
        );
        drop(new);
        assert!(super::RELAY.read().unwrap().is_none());
    }

    #[test]
    fn automatic_windows_and_macos_clients_get_the_runtime_relay() {
        let empty = NetworkSettings::default();
        let projected = project(&empty, RELAY, true);
        assert_eq!(projected.https_proxy.as_deref(), Some(RELAY));
        assert!(
            projected
                .no_proxy
                .as_deref()
                .is_some_and(|v| v.contains("127.0.0.1"))
        );
        assert!(empty.is_empty());
    }

    #[test]
    fn manual_proxies_and_linux_settings_are_unchanged() {
        let manual =
            NetworkSettings::default().with("HTTPS_PROXY", Some("http://manual.test:8080".into()));
        let projected = project(&manual, RELAY, true);
        assert_eq!(projected.https_proxy, manual.https_proxy);
        assert_eq!(projected.http_proxy.as_deref(), Some(RELAY));
        let all = manual.with("ALL_PROXY", Some("http://fallback.test:8080".into()));
        assert_eq!(project(&all, RELAY, true), all);
        let empty = NetworkSettings::default();
        assert_eq!(project(&empty, RELAY, false), empty);
    }

    #[test]
    fn updater_route_needs_verified_settings_and_a_usable_https_path() {
        let empty = NetworkSettings::default();
        assert!(!route_ready_for(&empty, false, false));
        assert!(!route_ready_for(&empty, false, true));
        assert!(!route_ready_for(&empty, true, false));
        assert!(route_ready_for(&empty, true, true));
        let manual = empty
            .clone()
            .with("HTTPS_PROXY", Some("http://127.0.0.1:8080".into()));
        assert!(!route_ready_for(&manual, false, false));
        assert!(route_ready_for(&manual, true, false));
        let invalid = empty
            .clone()
            .with("HTTPS_PROXY", Some("not a proxy".into()));
        assert!(!route_ready_for(&invalid, true, true));
        let http_only = empty.with("HTTP_PROXY", Some("http://127.0.0.1:8080".into()));
        assert!(!route_ready_for(&http_only, true, false));
    }

    #[test]
    fn automatic_projection_preserves_certificates_and_bypass() {
        let source = NetworkSettings::default()
            .with("NO_PROXY", Some("inside.test".into()))
            .with("SSL_CERT_FILE", Some("/trusted.pem".into()));
        let projected = project(&source, RELAY, true);
        assert_eq!(projected.ssl_cert_file, source.ssl_cert_file);
        assert!(
            projected
                .no_proxy
                .as_deref()
                .is_some_and(|v| v.contains("inside.test"))
        );
        assert_eq!(projected.https_proxy.as_deref(), Some(RELAY));
    }

    async fn receipt(listener: tokio::net::TcpListener, body: &'static str) {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(stream.read_u8().await.unwrap());
        }
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    }

    #[tokio::test]
    async fn automatic_client_reaches_relay_instead_of_its_direct_destination() {
        let direct = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let direct_address = direct.local_addr().unwrap();
        let relay = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", relay.local_addr().unwrap());
        let direct_task = tokio::spawn(receipt(direct, "DIRECT"));
        let relay_task = tokio::spawn(receipt(relay, "RELAY"));
        let settings = project(&NetworkSettings::default(), &endpoint, true);
        let client = settings
            .apply(crate::tls_builder().no_proxy())
            .resolve("proxy-target.test", direct_address)
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap();
        let answer = client
            .get(format!(
                "http://proxy-target.test:{}/receipt",
                direct_address.port()
            ))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        direct_task.abort();
        relay_task.abort();
        assert_eq!(answer, "RELAY");
    }

    #[tokio::test]
    async fn native_automatic_client_without_relay_cannot_open_a_direct_socket() {
        for scheme in ["http", "https"] {
            let direct = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = direct.local_addr().unwrap();
            let client = apply_relay(
                &NetworkSettings::default(),
                crate::tls_builder().no_proxy(),
                None,
                true,
            )
            .resolve("missing-relay.test", address)
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap();
            assert!(
                client
                    .get(format!("{scheme}://missing-relay.test:{}/", address.port()))
                    .send()
                    .await
                    .is_err()
            );
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(100), direct.accept())
                    .await
                    .is_err(),
                "automatic {scheme} route sent a direct request before the relay existed"
            );
        }
    }

    #[tokio::test]
    async fn native_client_without_relay_can_reach_loopback() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let client = apply_relay(
            &NetworkSettings::default(),
            crate::tls_builder().no_proxy(),
            None,
            true,
        )
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();
        let receipt = tokio::spawn(receipt(listener, "LOOPBACK"));
        let answer = client
            .get(format!("http://{address}/"))
            .send()
            .await
            .unwrap();
        assert_eq!(answer.text().await.unwrap(), "LOOPBACK");
        receipt.await.unwrap();
    }

    #[tokio::test]
    async fn native_client_without_relay_uses_manual_http_proxy() {
        let proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", proxy.local_addr().unwrap());
        let settings = NetworkSettings::default().with("HTTP_PROXY", Some(endpoint));
        let client = apply_relay(&settings, crate::tls_builder().no_proxy(), None, true)
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap();
        let receipt = tokio::spawn(receipt(proxy, "MANUAL"));
        let answer = client
            .get("http://manual-route.test/")
            .send()
            .await
            .unwrap();
        assert_eq!(answer.text().await.unwrap(), "MANUAL");
        receipt.await.unwrap();
    }

    #[tokio::test]
    async fn native_retained_client_reopens_relay_after_route_change() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let route_changed = Arc::new(AtomicBool::new(false));
        let changed_for_server = Arc::clone(&route_changed);
        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let body = if changed_for_server.load(Ordering::SeqCst) {
                    "NEW"
                } else {
                    "OLD"
                };
                tokio::spawn(async move {
                    let mut pending = Vec::new();
                    let mut chunk = [0_u8; 1024];
                    while let Ok(size) = socket.read(&mut chunk).await {
                        if size == 0 {
                            break;
                        }
                        pending.extend_from_slice(&chunk[..size]);
                        while let Some(end) =
                            pending.windows(4).position(|part| part == b"\r\n\r\n")
                        {
                            pending.drain(..end + 4);
                            if socket
                                .write_all(
                                    format!(
                                        "HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: keep-alive\r\n\r\n{body}"
                                    )
                                    .as_bytes(),
                                )
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                });
            }
        });
        let relay = Arc::new(super::Relay {
            endpoint: format!("http://{address}"),
            active: AtomicBool::new(true),
        });
        let owner = SystemProxy(Arc::clone(&relay));
        let client = apply_relay(
            &NetworkSettings::default(),
            crate::tls_builder().no_proxy(),
            Some(relay),
            true,
        )
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();
        let first = client
            .get("http://route-refresh.test/resource")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(first, "OLD");
        route_changed.store(true, Ordering::SeqCst);
        let second = client
            .get("http://route-refresh.test/resource")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(
            second, "NEW",
            "the retained client reused its old relay route"
        );
        owner.clear();
        server.abort();
    }

    #[tokio::test]
    async fn native_https_only_manual_proxy_does_not_allow_direct_http() {
        let direct = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = direct.local_addr().unwrap();
        let settings =
            NetworkSettings::default().with("HTTPS_PROXY", Some("http://127.0.0.1:9".into()));
        let client = apply_relay(&settings, crate::tls_builder().no_proxy(), None, true)
            .resolve("automatic-http.test", address)
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap();
        assert!(
            client
                .get(format!("http://automatic-http.test:{}/", address.port()))
                .send()
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), direct.accept())
                .await
                .is_err(),
            "the missing automatic HTTP route opened a direct socket"
        );
    }

    #[tokio::test]
    async fn retained_client_reaches_loopback_after_relay_owner_stops() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let relay = Arc::new(super::Relay {
            endpoint: "http://127.0.0.1:9".into(),
            active: AtomicBool::new(true),
        });
        let owner = SystemProxy(Arc::clone(&relay));
        let client = apply_relay(
            &NetworkSettings::default(),
            crate::tls_builder().no_proxy(),
            Some(relay),
            true,
        )
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();
        owner.clear();
        let receipt = tokio::spawn(receipt(listener, "LOOPBACK"));
        let answer = client
            .get(format!("http://{address}/"))
            .send()
            .await
            .unwrap();
        assert_eq!(answer.text().await.unwrap(), "LOOPBACK");
        receipt.await.unwrap();
    }

    #[tokio::test]
    async fn retained_client_uses_manual_http_after_relay_owner_stops() {
        let proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", proxy.local_addr().unwrap());
        let settings = NetworkSettings::default().with("HTTP_PROXY", Some(endpoint));
        let relay = Arc::new(super::Relay {
            endpoint: "http://127.0.0.1:9".into(),
            active: AtomicBool::new(true),
        });
        let owner = SystemProxy(Arc::clone(&relay));
        let client = apply_relay(
            &settings,
            crate::tls_builder().no_proxy(),
            Some(relay),
            true,
        )
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();
        owner.clear();
        let receipt = tokio::spawn(receipt(proxy, "MANUAL"));
        let answer = client
            .get("http://manual-route.test/")
            .send()
            .await
            .unwrap();
        assert_eq!(answer.text().await.unwrap(), "MANUAL");
        receipt.await.unwrap();
    }

    #[tokio::test]
    async fn native_manual_https_proxy_still_receives_the_request_without_a_relay() {
        let direct = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = direct.local_addr().unwrap();
        let proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let settings = NetworkSettings::default().with(
            "HTTPS_PROXY",
            Some(format!("http://{}", proxy.local_addr().unwrap())),
        );
        let client = apply_relay(&settings, crate::tls_builder().no_proxy(), None, true)
            .resolve("manual-route.test", address)
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap();
        assert!(
            client
                .get(format!("https://manual-route.test:{}/", address.port()))
                .send()
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), proxy.accept())
                .await
                .is_ok(),
            "the manual proxy did not receive the HTTPS connection"
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), direct.accept())
                .await
                .is_err(),
            "the manual proxy was bypassed"
        );
    }

    #[tokio::test]
    async fn explicit_no_proxy_still_bypasses_the_relay() {
        let direct = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = direct.local_addr().unwrap();
        let relay_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay = Arc::new(super::Relay {
            endpoint: format!("http://{}", relay_listener.local_addr().unwrap()),
            active: AtomicBool::new(true),
        });
        let settings = NetworkSettings::default().with("NO_PROXY", Some("no-proxy.test".into()));
        let client = apply_relay(
            &settings,
            crate::tls_builder().no_proxy(),
            Some(relay),
            true,
        )
        .resolve("no-proxy.test", address)
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();
        let direct_task = tokio::spawn(receipt(direct, "DIRECT"));
        let body = client
            .get(format!("http://no-proxy.test:{}/", address.port()))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        direct_task.await.unwrap();
        assert_eq!(body, "DIRECT");
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(100),
                relay_listener.accept()
            )
            .await
            .is_err(),
            "an explicit no-proxy choice went to the relay"
        );
    }

    #[tokio::test]
    async fn update_client_rechecks_route_authorization_before_connecting() {
        let _serial = GLOBAL_ROUTE_TEST.lock().await;
        let direct = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = direct.local_addr().unwrap();
        super::super::install_for_launch(&NetworkSettings::default(), &NetworkSettings::default());
        super::super::authorize_update_route(true);
        let _owner = set_system_proxy(format!("http://{address}"));
        let client = super::super::current()
            .apply_update(crate::tls_builder().no_proxy())
            .resolve("route-transition.test", address)
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap();
        super::super::authorize_update_route(false);
        assert!(
            client
                .get(format!("http://route-transition.test:{}/", address.port()))
                .send()
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), direct.accept())
                .await
                .is_err(),
            "a client built before the vault transition opened a socket afterwards"
        );
        super::super::set_current(NetworkSettings::default());
    }

    #[cfg(windows)]
    #[test]
    fn windows_automatic_route_is_projected() {
        assert_eq!(
            project(&NetworkSettings::default(), RELAY, cfg!(windows))
                .https_proxy
                .as_deref(),
            Some(RELAY)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_automatic_route_is_projected() {
        assert_eq!(
            project(
                &NetworkSettings::default(),
                RELAY,
                cfg!(target_os = "macos")
            )
            .https_proxy
            .as_deref(),
            Some(RELAY)
        );
    }
}
