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
        ProbeKind::ProxyLookupFailed => NetworkTestKind::ProxyLookupFailed,
        ProbeKind::ProxySignInNeeded => NetworkTestKind::ProxySignInNeeded,
        ProbeKind::CertificateNotTrusted => NetworkTestKind::CertificateNotTrusted,
        ProbeKind::NameNotFound => NetworkTestKind::NameNotFound,
        ProbeKind::ConnectFailed => NetworkTestKind::ConnectFailed,
        ProbeKind::TimedOut => NetworkTestKind::TimedOut,
        ProbeKind::Failed => NetworkTestKind::Failed,
    }
}

/// A reached result is accepted only when the route selected for that request is known.
pub(crate) fn verified_probe(probe: Probe, observed_proxy: Option<bool>) -> Probe {
    if probe.kind == ProbeKind::Reached && observed_proxy.is_none() {
        Probe {
            kind: ProbeKind::Failed,
            status: None,
        }
    } else {
        probe
    }
}

/// The test's result in words. `settings` are the ones that were tried; `overridden` the labels of the settings this
/// computer's environment overrides. `observed_proxy` is the request's actual route, never inferred from fields.
pub(crate) fn describe(
    probe: Probe,
    settings: &NetworkSettings,
    overridden: Vec<String>,
    observed_proxy: Option<bool>,
) -> NetworkTestView {
    let through_proxy = observed_proxy == Some(true);
    let has_certificate = settings.ssl_cert_file.is_some() || settings.ssl_cert_dir.is_some();
    let (summary, next_step): (&str, Option<&str>) = match probe.kind {
        ProbeKind::Reached if through_proxy => (
            "Mistral answered through the proxy. Vibe Work can connect this way.",
            None,
        ),
        ProbeKind::Reached => (
            "Mistral answered directly. Vibe Work can connect this way.",
            None,
        ),
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
        ProbeKind::ProxyLookupFailed => (
            "Vibe Work could not read this computer's proxy setup, so it did not connect.",
            Some("Check the system proxy or PAC settings, or enter a proxy in Settings > Network."),
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
    use vw_mistral::network::{NetworkSettings, Probe, ProbeKind, classify_guarded_probe};

    use super::{describe, verified_probe};

    const KINDS: [ProbeKind; 10] = [
        ProbeKind::Reached,
        ProbeKind::ProxyUnreachable,
        ProbeKind::ProxyRefused,
        ProbeKind::ProxyLookupFailed,
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
            let view = describe(Probe { kind, status: None }, &settings, Vec::new(), None);
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
            describe(probe, &bare, vec![], None)
                .next_step
                .expect("step")
                .contains("Choose your company's")
        );
        assert!(
            describe(probe, &chosen, vec![], None)
                .next_step
                .expect("step")
                .contains("does not cover")
        );
    }

    #[test]
    fn system_proxy_success_reports_the_observed_proxy_route() {
        let view = describe(
            Probe {
                kind: ProbeKind::Reached,
                status: Some(401),
            },
            &NetworkSettings::default(),
            vec![],
            Some(true),
        );
        assert!(view.through_proxy);
        assert_eq!(
            view.summary,
            "Mistral answered through the proxy. Vibe Work can connect this way."
        );
    }

    #[test]
    fn direct_success_reports_the_observed_direct_route_even_with_proxy_fields() {
        let settings = NetworkSettings::default()
            .with("HTTPS_PROXY", Some("http://proxy.corp:8080".to_owned()));
        let view = describe(
            Probe {
                kind: ProbeKind::Reached,
                status: Some(401),
            },
            &settings,
            vec![],
            Some(false),
        );
        assert!(!view.through_proxy);
        assert_eq!(
            view.summary,
            "Mistral answered directly. Vibe Work can connect this way."
        );
    }

    #[test]
    fn default_settings_direct_success_names_the_route_in_visible_text() {
        let view = describe(
            Probe {
                kind: ProbeKind::Reached,
                status: Some(401),
            },
            &NetworkSettings::default(),
            vec![],
            Some(false),
        );
        assert!(!view.through_proxy);
        assert_eq!(
            view.summary,
            "Mistral answered directly. Vibe Work can connect this way."
        );
    }

    #[test]
    fn local_forwarder_error_is_not_a_reached_target() {
        let local_error = Probe {
            kind: ProbeKind::Reached,
            status: Some(502),
        };
        assert_eq!(
            verified_probe(local_error, None),
            Probe {
                kind: ProbeKind::Failed,
                status: None,
            }
        );
        for through_proxy in [false, true] {
            assert_eq!(
                verified_probe(local_error, Some(through_proxy)),
                local_error
            );
        }
    }

    #[test]
    fn lookup_failure_and_proxy_refusal_name_different_causes() {
        let settings = NetworkSettings::default();
        let lookup = describe(
            Probe {
                kind: ProbeKind::ProxyLookupFailed,
                status: None,
            },
            &settings,
            vec![],
            None,
        );
        assert_eq!(
            lookup.summary,
            "Vibe Work could not read this computer's proxy setup, so it did not connect."
        );
        assert_eq!(
            lookup.next_step.as_deref(),
            Some("Check the system proxy or PAC settings, or enter a proxy in Settings > Network.")
        );
        let refused = describe(
            Probe {
                kind: ProbeKind::ProxyRefused,
                status: None,
            },
            &settings,
            vec![],
            None,
        );
        assert_eq!(
            refused.summary,
            "The proxy answered, but would not pass the request on to Mistral."
        );
    }

    #[test]
    fn dead_proxy_and_answered_proxy_refusal_use_their_existing_sentences() {
        let settings = NetworkSettings::default();
        let raw = Probe {
            kind: ProbeKind::ProxyRefused,
            status: None,
        };
        let dead = describe(
            classify_guarded_probe(raw, false, true),
            &settings,
            vec![],
            None,
        );
        assert_eq!(dead.summary, "Vibe Work could not reach the proxy.");
        let answered = describe(
            classify_guarded_probe(raw, false, false),
            &settings,
            vec![],
            None,
        );
        assert_eq!(
            answered.summary,
            "The proxy answered, but would not pass the request on to Mistral."
        );
    }
}
