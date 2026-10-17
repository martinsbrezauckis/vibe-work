//! The words of the Settings > Network connection test: one plain sentence for what happened and one for what to
//! check next, chosen from vw-mistral's [`ProbeKind`].
//!
//! Depends on: vw-mistral (`network`: the probe), `network_view`. Called by: `network`.
//! Invariants: the sentences never repeat an address or a path (a proxy address can carry a sign-in); the next step
//! names things the user can act on (the proxy address, the certificate file, the IT department), never an error
//! code.

use vw_mistral::network::{NetworkSettings, Probe, ProbeKind};

use crate::network_view::{NetworkTestKind, NetworkTestView};

fn kind_of(kind: ProbeKind) -> NetworkTestKind {
    match kind {
        ProbeKind::Reached => NetworkTestKind::Reached,
        ProbeKind::ProxyUnreachable => NetworkTestKind::ProxyUnreachable,
        ProbeKind::ProxyRefused => NetworkTestKind::ProxyRefused,
        ProbeKind::ProxySignInNeeded => NetworkTestKind::ProxySignInNeeded,
        ProbeKind::CertificateNotTrusted => NetworkTestKind::CertificateNotTrusted,
        ProbeKind::NameNotFound => NetworkTestKind::NameNotFound,
        ProbeKind::ConnectFailed => NetworkTestKind::ConnectFailed,
        ProbeKind::TimedOut => NetworkTestKind::TimedOut,
        ProbeKind::Failed => NetworkTestKind::Failed,
    }
}

/// The test's result in words. `settings` are the ones that were tried; `overridden` the labels of the settings this
/// computer's environment overrides.
pub(crate) fn describe(
    probe: Probe,
    settings: &NetworkSettings,
    overridden: Vec<String>,
) -> NetworkTestView {
    let through_proxy = settings.has_proxy();
    let has_certificate = settings.ssl_cert_file.is_some() || settings.ssl_cert_dir.is_some();
    let (summary, next_step): (&str, Option<&str>) = match probe.kind {
        ProbeKind::Reached if through_proxy => (
            "Mistral answered through the proxy. Vibe Work can connect this way.",
            None,
        ),
        ProbeKind::Reached => ("Mistral answered. Vibe Work can connect this way.", None),
        ProbeKind::ProxyUnreachable => (
            "Vibe Work could not reach the proxy.",
            Some(
                "Check the proxy address and its port, and that this computer is on the company network or VPN.",
            ),
        ),
        ProbeKind::ProxyRefused => (
            "The proxy answered, but would not pass the request on to Mistral.",
            Some(
                "Ask your IT department to allow api.mistral.ai (and its subdomains) through the proxy.",
            ),
        ),
        ProbeKind::ProxySignInNeeded => (
            "The proxy asks for a user name and password.",
            Some(
                "Type them in the address: http://user:password@host:port. A proxy that signs you in with your Windows or company account may not work with Vibe; your IT department can say which kind it is.",
            ),
        ),
        ProbeKind::CertificateNotTrusted if has_certificate => (
            "The connection was made, but its certificate is still not trusted.",
            Some(
                "The certificate you chose does not cover this connection. Ask your IT department for the certificate their proxy uses (a .pem file).",
            ),
        ),
        ProbeKind::CertificateNotTrusted => (
            "The connection was made, but its certificate is not trusted.",
            Some(
                "Companies that inspect web traffic use their own certificate. Choose your company's certificate file (.pem) in the certificate field, or ask your IT department for it.",
            ),
        ),
        ProbeKind::NameNotFound => (
            "Mistral's address could not be found.",
            Some(
                "This computer may have no direct internet access. If your company uses a proxy, enter its address in the proxy field.",
            ),
        ),
        ProbeKind::ConnectFailed => (
            "Vibe Work could not connect to Mistral.",
            Some(
                "This computer may have no direct internet access. If your company uses a proxy, enter its address in the proxy field.",
            ),
        ),
        ProbeKind::TimedOut => (
            "There was no answer in time.",
            Some("Check the network and the proxy, then try again."),
        ),
        ProbeKind::Failed => (
            "The test failed for a reason Vibe Work could not tell.",
            Some("Check the proxy address and the certificate, then try again."),
        ),
    };
    NetworkTestView {
        kind: kind_of(probe.kind),
        reached: probe.kind == ProbeKind::Reached,
        summary: summary.to_owned(),
        next_step: next_step.map(str::to_owned),
        through_proxy,
        overridden_by_environment: overridden,
    }
}

#[cfg(test)]
mod tests {
    use vw_mistral::network::{NetworkSettings, Probe, ProbeKind};

    use super::describe;

    const KINDS: [ProbeKind; 9] = [
        ProbeKind::Reached,
        ProbeKind::ProxyUnreachable,
        ProbeKind::ProxyRefused,
        ProbeKind::ProxySignInNeeded,
        ProbeKind::CertificateNotTrusted,
        ProbeKind::NameNotFound,
        ProbeKind::ConnectFailed,
        ProbeKind::TimedOut,
        ProbeKind::Failed,
    ];

    #[test]
    fn every_failure_has_a_next_step_and_no_success_does() {
        let settings = NetworkSettings::default().with(
            "HTTPS_PROXY",
            Some("http://user:secret@proxy.corp:8080".to_owned()),
        );
        for kind in KINDS {
            let view = describe(Probe { kind, status: None }, &settings, Vec::new());
            assert_eq!(view.reached, kind == ProbeKind::Reached, "{kind:?}");
            assert_eq!(view.next_step.is_none(), view.reached, "{kind:?}");
            for text in [Some(&view.summary), view.next_step.as_ref()]
                .into_iter()
                .flatten()
            {
                assert!(
                    !text.contains("secret") && !text.contains("proxy.corp"),
                    "{text}"
                );
            }
        }
    }

    #[test]
    fn an_untrusted_certificate_reads_differently_when_one_was_already_chosen() {
        let bare = NetworkSettings::default();
        let chosen = bare
            .clone()
            .with("SSL_CERT_FILE", Some("/ca.pem".to_owned()));
        let probe = Probe {
            kind: ProbeKind::CertificateNotTrusted,
            status: None,
        };
        assert!(
            describe(probe, &bare, vec![])
                .next_step
                .expect("step")
                .contains("Choose your company's")
        );
        assert!(
            describe(probe, &chosen, vec![])
                .next_step
                .expect("step")
                .contains("does not cover")
        );
    }
}
