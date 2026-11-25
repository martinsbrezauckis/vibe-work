//! Readable reasons for a server that cannot be used: Vibe reports a failed connection as raw text (or only as
//! "needs sign-in"); this turns the causes a person can fix into one sentence with its action.
//!
//! Depends on: nothing. Called by: `connectors_project` (Vibe's text), `mcp_keys` (the forwarder's own record).
//! Invariants: only the cause and the action are stated, never a value from the raw text (it may quote an
//! address with a query or a header); text that is not recognised stays as Vibe wrote it, with no kind.

use std::collections::HashMap;
use std::time::Duration;

use vw_keyproxy::McpProblem;
use vw_mistral::network::{NetworkSettings, ProbeKind};

use crate::connectors_view::ServerView;
use crate::core::{Core, Shared};

/// Vibe's transient notice for a server that is configured but not yet running in the session: not a failure.
pub(crate) const NOT_ACTIVE: &str = "MCP source configuration is not active in this session";

/// What a probe found at an address, kept for [`PROBE_REUSE`]: a cause, or nothing wrong.
#[derive(Debug, Clone)]
pub(crate) struct Probed(pub(crate) Option<Reason>);

/// A failure the person can act on: the machine-readable `kind` the Tools page reads, and the sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Reason {
    pub(crate) kind: &'static str,
    pub(crate) text: String,
}

fn reason(kind: &'static str, text: &str) -> Reason {
    Reason {
        kind,
        text: text.to_owned(),
    }
}

/// The server's certificate is not trusted here.
pub(crate) fn certificate() -> Reason {
    reason(
        "certificate",
        "This server's certificate is not trusted on this computer. If it uses a private certificate authority, add its certificate in Settings > Network (Company certificate file), then quit and reopen Vibe Work.",
    )
}

/// Nothing answered at the address.
pub(crate) fn unreachable() -> Reason {
    reason(
        "unreachable",
        "Vibe could not reach this server. Check that it is running and that the address and port are right, then refresh.",
    )
}

/// No answer in time.
pub(crate) fn timed_out() -> Reason {
    reason(
        "timeout",
        "This server did not answer in time. Check that it is running, then refresh.",
    )
}

/// The server answered the API key with 401 or 403.
pub(crate) fn key_rejected(status: u16) -> Reason {
    Reason {
        kind: "keyRejected",
        text: format!(
            "This server rejected the API key (HTTP {status}). Check the key's value and the header it goes in, or remove the server and add it again with another key."
        ),
    }
}

/// A server added without sign-in answered that it needs credentials.
pub(crate) fn needs_credentials() -> Reason {
    reason(
        "needsCredentials",
        "This server asks for credentials, but it was added without a sign-in. Remove it and add it again with an API key or with Sign in.",
    )
}

fn forbidden() -> Reason {
    reason(
        "forbidden",
        "This server refused access (HTTP 403). Check that your account may use it; if it needs an API key or a sign-in, remove it and add it again with that choice.",
    )
}

/// What `raw` (Vibe's error text for a server) means for a server whose sign-in kind is `sign_in`
/// (`oauth`, `static`/`key`, `none`). `None`: not recognised, or Vibe's transient "not active yet" notice.
pub(crate) fn explain(raw: &str, sign_in: &str) -> Option<Reason> {
    let text = raw.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| text.contains(n));
    if has(&[
        "certificate",
        "self-signed",
        "self signed",
        "unknown issuer",
        "unknownissuer",
        "ssl",
        "tls handshake",
    ]) {
        return Some(certificate());
    }
    if has(&[
        "connection refused",
        "connecterror",
        "connect error",
        "error trying to connect",
        "could not connect",
        "name or service not known",
        "no such host",
        "failed to lookup",
        "dns",
        "network is unreachable",
        "no route to host",
        "vibe_work_upstream_unreachable",
    ]) {
        return Some(unreachable());
    }
    if has(&["timed out", "timeout"]) {
        return Some(timed_out());
    }
    let unauthorized = has(&["401", "unauthorized"]);
    let denied = has(&["403", "forbidden"]);
    match (sign_in, unauthorized, denied) {
        ("key" | "static", true, _) => Some(key_rejected(401)),
        ("key" | "static", _, true) => Some(key_rejected(403)),
        ("none", true, _) => Some(needs_credentials()),
        ("none", _, true) => Some(forbidden()),
        _ => None,
    }
}

/// The text and kind to show for a server: Vibe's own `error` (or its discovery error), made readable when it is
/// recognised; for a server without sign-in that Vibe marks `needs_auth`, the "added without a sign-in" action.
pub(crate) fn for_server(
    raw: Option<&str>,
    status: Option<&str>,
    sign_in: &str,
) -> (Option<String>, Option<&'static str>) {
    let raw = raw.filter(|r| !r.is_empty() && *r != NOT_ACTIVE);
    if let Some(found) = raw.and_then(|r| explain(r, sign_in)) {
        return (Some(found.text), Some(found.kind));
    }
    if sign_in == "none" && status == Some("needs_auth") {
        let found = needs_credentials();
        return (Some(found.text), Some(found.kind));
    }
    (raw.map(str::to_owned), None)
}

/// How long a probe's answer is reused, and the longest a probe may hold up the Tools page.
pub(crate) const PROBE_REUSE: Duration = Duration::from_secs(30);
const PROBE_LIMIT: Duration = Duration::from_secs(3);

/// Vibe reports a failed direct connection as a bare code (`mcp_transport_failed`), so Vibe Work asks the address
/// itself once: the cause a person can fix (certificate, nothing there, no answer, or the server wants credentials).
/// `None` when the address answers without a recognised problem.
pub(crate) async fn probe_reason(
    settings: &NetworkSettings,
    address: &str,
    sign_in: &str,
) -> Option<Reason> {
    let probe = tokio::time::timeout(PROBE_LIMIT, vw_mistral::network::probe(settings, address))
        .await
        .unwrap_or(vw_mistral::network::Probe {
            kind: ProbeKind::TimedOut,
            status: None,
        });
    match (probe.kind, probe.status) {
        (ProbeKind::CertificateNotTrusted, _) => Some(certificate()),
        (ProbeKind::TimedOut, _) => Some(timed_out()),
        (
            ProbeKind::NameNotFound
            | ProbeKind::ConnectFailed
            | ProbeKind::ProxyUnreachable
            | ProbeKind::ProxyRefused,
            _,
        ) => Some(unreachable()),
        (ProbeKind::Reached, Some(401)) if sign_in == "none" => Some(needs_credentials()),
        (ProbeKind::Reached, Some(403)) if sign_in == "none" => Some(forbidden()),
        _ => None,
    }
}

impl Core {
    /// Gives each direct server (no sign-in, or sign-in) that Vibe reports as failed without a cause the cause its
    /// address shows, probing at most once per [`PROBE_REUSE`] per server; the probes run together.
    pub(crate) async fn explain_direct_failures(&self, servers: &mut [ServerView]) {
        let wanted: Vec<(String, String, String)> = servers
            .iter()
            .filter(|s| {
                s.configured
                    && !s.off
                    && s.error.is_some()
                    && s.error_kind.is_none()
                    && matches!(s.sign_in.as_str(), "none" | "oauth")
            })
            .filter_map(|s| Some((s.name.clone(), s.address.clone()?, s.sign_in.clone())))
            .collect();
        if wanted.is_empty() {
            return;
        }
        let settings = self.server_probe_settings().await;
        let mut found: HashMap<String, Option<Reason>> = HashMap::default();
        let mut running = tokio::task::JoinSet::new();
        for (name, address, sign_in) in wanted {
            let key = format!("{name} {address}");
            if let Some(cached) = self.shared.connectors.cached_probe(&key) {
                found.insert(name, cached.0);
                continue;
            }
            let Some(settings) = settings.clone() else {
                continue;
            };
            running.spawn(async move {
                let reason = probe_reason(&settings, &address, &sign_in).await;
                (name, key, reason)
            });
        }
        while let Some(Ok((name, key, reason))) = running.join_next().await {
            self.shared
                .connectors
                .keep_probe(key, Probed(reason.clone()));
            found.insert(name, reason);
        }
        for server in servers.iter_mut() {
            if let Some(Some(reason)) = found.remove(&server.name) {
                server.error = Some(reason.text);
                server.error_kind = Some(reason.kind.to_owned());
            }
        }
    }
}

/// For each key server whose last request to its address failed, the reason (the forwarder records it): it
/// replaces Vibe's text, which only says the connection failed. Runs after the session's state is merged.
pub(crate) async fn merge_key_problems(shared: &Shared, servers: &mut [ServerView]) {
    let (Some(keys), Ok(list)) = (
        shared.config.keys.as_ref(),
        crate::mcp_keys::bindings(shared).await,
    ) else {
        return;
    };
    for server in servers.iter_mut() {
        let Some(problem) = list
            .iter()
            .find(|b| b.server == server.name)
            .and_then(|b| keys.mcp_problem(&b.id))
        else {
            continue;
        };
        let found = match problem {
            McpProblem::CertificateNotTrusted => certificate(),
            McpProblem::Unreachable => unreachable(),
            McpProblem::TimedOut => timed_out(),
            McpProblem::KeyRejected { status } => key_rejected(status),
        };
        server.error = Some(found.text);
        server.error_kind = Some(found.kind.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_cause_has_its_own_kind() {
        let kind = |raw, sign_in| explain(raw, sign_in).map(|r| r.kind);
        assert_eq!(
            kind(
                "error sending request: invalid peer certificate: UnknownIssuer",
                "none"
            ),
            Some("certificate")
        );
        assert_eq!(
            kind("ConnectError: Connection refused", "none"),
            Some("unreachable")
        );
        assert_eq!(kind("request timed out", "oauth"), Some("timeout"));
        assert_eq!(kind("HTTP 401 Unauthorized", "key"), Some("keyRejected"));
        assert_eq!(
            kind("HTTP 401 Unauthorized", "none"),
            Some("needsCredentials")
        );
        assert_eq!(kind("HTTP 403 Forbidden", "none"), Some("forbidden"));
        // A sign-in server's 401 is Vibe's own business (the sign-in flow), not rewritten here.
        assert_eq!(kind("HTTP 401 Unauthorized", "oauth"), None);
        assert_eq!(kind("something else", "none"), None);
    }

    #[test]
    fn the_not_active_notice_is_no_failure_and_needs_auth_without_sign_in_is_explained() {
        assert_eq!(for_server(Some(NOT_ACTIVE), None, "none"), (None, None));
        let (text, kind) = for_server(None, Some("needs_auth"), "none");
        assert_eq!(kind, Some("needsCredentials"));
        assert!(text.unwrap().contains("API key"));
        assert_eq!(for_server(None, Some("needs_auth"), "oauth"), (None, None));
    }

    #[tokio::test]
    async fn a_closed_port_is_unreachable_and_an_open_server_answering_401_needs_credentials() {
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}/mcp", closed.local_addr().unwrap());
        drop(closed);
        let settings = NetworkSettings::default();
        let reason = probe_reason(&settings, &address, "none").await.unwrap();
        assert_eq!(reason.kind, "unreachable");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}/mcp", listener.local_addr().unwrap());
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Ok((mut tcp, _)) = listener.accept().await {
                let mut buf = [0u8; 512];
                let _ = tcp.read(&mut buf).await;
                let _ = tcp
                    .write_all(b"HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                    .await;
            }
        });
        let reason = probe_reason(&settings, &address, "none").await.unwrap();
        assert_eq!(reason.kind, "needsCredentials");
        // A sign-in server's 401 is its sign-in flow, not a failure to explain here.
        assert!(probe_reason(&settings, &address, "oauth").await.is_none());
    }
}
