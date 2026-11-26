//! The connection test of Settings: one plain GET, without a key, through the settings being tested.
//!
//! Depends on: reqwest, `client` (`tls_builder`), `network` (the settings). Called by: vw-core-api (`network`).
//! Invariants:
//! - No key and no cookie is sent; the request is a GET of one address the caller names (normally Mistral's
//!   model list, which answers 401 without a key: that answer proves the way through works).
//! - The outcome is a kind and a status, never the text of an error: reqwest's messages repeat the address.
//! - Any HTTP answer from the target counts as reached, except 407 (the proxy wants a sign-in).

use std::error::Error as _;
use std::time::Duration;

use super::NetworkSettings;

/// What the test found out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeKind {
    /// The target answered (the status says what).
    Reached,
    /// The proxy's own address could not be reached (wrong name or port, or it is down).
    ProxyUnreachable,
    /// The proxy answered but refused to pass the request on.
    ProxyRefused,
    /// The system proxy/PAC lookup failed before any proxy was contacted.
    ProxyLookupFailed,
    /// The proxy wants a user name and password (407).
    ProxySignInNeeded,
    /// The connection worked up to the TLS check, which failed: the certificate is not trusted.
    CertificateNotTrusted,
    /// Without a proxy: the target's name could not be found.
    NameNotFound,
    /// Without a proxy: the connection to the target failed.
    ConnectFailed,
    /// No answer in time.
    TimedOut,
    /// Anything else.
    Failed,
}

/// The test's result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probe {
    /// What happened.
    pub kind: ProbeKind,
    /// The target's HTTP status when it answered.
    pub status: Option<u16>,
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(15);

/// Requests `url` once with a client built from `settings` (no redirects, 15 seconds in all).
pub async fn probe(settings: &NetworkSettings, url: &str) -> Probe {
    let built = settings
        .apply(crate::tls_builder())
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TOTAL_TIMEOUT)
        .build();
    let Ok(client) = built else {
        return Probe {
            kind: ProbeKind::Failed,
            status: None,
        };
    };
    match client.get(url).send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            Probe {
                kind: if status == 407 {
                    ProbeKind::ProxySignInNeeded
                } else {
                    ProbeKind::Reached
                },
                status: Some(status),
            }
        }
        Err(error) => Probe {
            kind: classify_request_error(&error, settings.has_proxy()),
            status: None,
        },
    }
}

/// The lower-case text of an error and everything under it.
fn chain_text(error: &reqwest::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(inner) = source {
        text.push_str(" | ");
        text.push_str(&inner.to_string());
        source = inner.source();
    }
    text.to_ascii_lowercase()
}

/// Sorts a failed request into a [`ProbeKind`] (shared with the MCP forwarder, which reports the same causes).
#[must_use]
pub fn classify_request_error(error: &reqwest::Error, via_proxy: bool) -> ProbeKind {
    if error.is_timeout() {
        return ProbeKind::TimedOut;
    }
    let text = chain_text(error);
    let has = |needles: &[&str]| needles.iter().any(|n| text.contains(n));
    if text.contains("proxy authorization required") {
        ProbeKind::ProxySignInNeeded
    } else if text.contains("failed to create underlying connection") {
        // hyper-util's tunnel wraps "could not connect to the proxy itself" in a tunnel error: not a refusal.
        ProbeKind::ProxyUnreachable
    } else if has(&["tunnel error"]) {
        ProbeKind::ProxyRefused
    } else if has(&[
        "certificate",
        "unknownissuer",
        "unknown issuer",
        "untrusted",
        "self-signed",
        "self signed",
        "bad issuer",
    ]) {
        ProbeKind::CertificateNotTrusted
    } else if !via_proxy
        && has(&[
            "dns error",
            "failed to lookup",
            "name or service not known",
            "no such host",
            "nodename nor servname",
        ])
    {
        ProbeKind::NameNotFound
    } else if error.is_connect() {
        if via_proxy {
            ProbeKind::ProxyUnreachable
        } else {
            ProbeKind::ConnectFailed
        }
    } else {
        ProbeKind::Failed
    }
}

/// A temporary network-test guard can identify a discovery or upstream connection failure that reqwest reports
/// only as a generic CONNECT failure. The guard's per-test refusal signals are read after the request.
#[must_use]
pub fn classify_guarded_probe(
    probe: Probe,
    proxy_lookup_failed: bool,
    proxy_connect_failed: bool,
) -> Probe {
    let guard_502 = probe.kind == ProbeKind::ProxyRefused
        || (probe.kind == ProbeKind::Reached && probe.status == Some(502));
    if proxy_lookup_failed && guard_502 {
        Probe {
            kind: ProbeKind::ProxyLookupFailed,
            status: None,
        }
    } else if proxy_connect_failed && guard_502 {
        Probe {
            kind: ProbeKind::ProxyUnreachable,
            status: None,
        }
    } else {
        probe
    }
}

#[cfg(test)]
mod lookup_failure_tests {
    use super::{NetworkSettings, Probe, ProbeKind, classify_guarded_probe, probe};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    #[test]
    fn discovery_and_real_proxy_refusal_have_different_outcomes() {
        let refused = Probe {
            kind: ProbeKind::ProxyRefused,
            status: None,
        };
        assert_eq!(classify_guarded_probe(refused, false, false), refused);
        assert_eq!(
            classify_guarded_probe(refused, true, false),
            Probe {
                kind: ProbeKind::ProxyLookupFailed,
                status: None,
            }
        );
        let unreachable = Probe {
            kind: ProbeKind::ProxyUnreachable,
            status: None,
        };
        assert_eq!(
            classify_guarded_probe(unreachable, true, false),
            unreachable
        );
        let forwarded_http = Probe {
            kind: ProbeKind::Reached,
            status: Some(502),
        };
        assert_eq!(
            classify_guarded_probe(forwarded_http, true, false).kind,
            ProbeKind::ProxyLookupFailed
        );
        assert_eq!(
            classify_guarded_probe(forwarded_http, false, false),
            forwarded_http
        );
    }

    #[test]
    fn dead_upstream_proxy_and_answered_refusal_have_different_outcomes() {
        let refused = Probe {
            kind: ProbeKind::ProxyRefused,
            status: None,
        };
        assert_eq!(
            classify_guarded_probe(refused, false, true),
            Probe {
                kind: ProbeKind::ProxyUnreachable,
                status: None,
            }
        );
        assert_eq!(classify_guarded_probe(refused, false, false), refused);
        assert_eq!(
            classify_guarded_probe(refused, true, false).kind,
            ProbeKind::ProxyLookupFailed
        );
        let forwarded_502 = Probe {
            kind: ProbeKind::Reached,
            status: Some(502),
        };
        assert_eq!(
            classify_guarded_probe(forwarded_502, false, true).kind,
            ProbeKind::ProxyUnreachable
        );
        let target_503 = Probe {
            kind: ProbeKind::Reached,
            status: Some(503),
        };
        assert_eq!(classify_guarded_probe(target_503, false, true), target_503);
    }

    #[tokio::test]
    async fn real_502_and_503_connect_refusals_stay_refused_without_a_guard_signal() {
        for response_status in [502, 503] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") {
                    head.push(stream.read_u8().await.unwrap());
                }
                assert!(
                    String::from_utf8_lossy(&head).starts_with("CONNECT allowed.test:443 HTTP/1.1")
                );
                let response =
                    format!("HTTP/1.1 {response_status} Refused\r\nContent-Length: 0\r\n\r\n");
                stream.write_all(response.as_bytes()).await.unwrap();
            });
            let settings =
                NetworkSettings::default().with("HTTPS_PROXY", Some(format!("http://{address}")));
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                probe(&settings, "https://allowed.test/"),
            )
            .await
            .unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(3), server)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(result.kind, ProbeKind::ProxyRefused);
            assert_eq!(classify_guarded_probe(result, false, false), result);
            assert_eq!(
                classify_guarded_probe(result, true, false).kind,
                ProbeKind::ProxyLookupFailed
            );
        }
    }
}
