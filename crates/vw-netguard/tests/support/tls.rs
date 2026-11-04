//! Fresh, short-lived proxy identities and rustls test-server acceptors on every target.
//! Called by proxy policy and connector tests; no real service keys or system trust changes.
#[path = "tls_key.rs"]
mod key;
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer,
    KeyUsagePurpose,
};
use std::sync::OnceLock;
use time::{Duration, OffsetDateTime};

pub struct Identity {
    pub cert: Vec<u8>,
    key: Vec<u8>,
}
pub struct Fixtures {
    pub ca: Vec<u8>,
    #[cfg(target_os = "linux")]
    #[allow(dead_code)] // Used by the policy subprocess, not by the connector unit test.
    pub ca_pem: String,
    pub good: Identity,
    pub wrong: Identity,
}
pub fn fixtures() -> &'static Fixtures {
    static FIXTURES: OnceLock<Fixtures> = OnceLock::new();
    FIXTURES.get_or_init(|| {
        let key = key::generate();
        let mut params = parameters("proxy fixture CA", vec![], &key);
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let ca = params.self_signed(&key).unwrap();
        let issuer = Issuer::new(params, key);
        Fixtures {
            good: identity(
                "vw-test-target.example",
                vec!["localhost".into(), "127.0.0.1".into()],
                &issuer,
            ),
            wrong: identity(
                "wrong-name.example",
                vec!["wrong-name.example".into()],
                &issuer,
            ),
            #[cfg(target_os = "linux")]
            ca_pem: ca.pem(),
            ca: ca.der().to_vec(),
        }
    })
}
fn parameters(name: &str, names: Vec<String>, key: &key::KeyPair) -> CertificateParams {
    #[cfg(target_os = "linux")]
    let _ = key; // rcgen's ring feature derives the key identifier internally.
    let mut params = CertificateParams::new(names).unwrap();
    params.not_before = OffsetDateTime::now_utc() - Duration::days(1);
    params.not_after = params.not_before + Duration::days(397);
    params.distinguished_name.push(DnType::CommonName, name);
    let mut serial = [0u8; 16];
    getrandom::fill(&mut serial).unwrap();
    params.serial_number = Some(serial.to_vec().into());
    // rcgen derives this with ring on Linux; the external signer supplies the same digest elsewhere.
    #[cfg(not(target_os = "linux"))]
    {
        use rcgen::PublicKeyData as _;
        use rsa::sha2::{Digest as _, Sha256};
        params.key_identifier_method =
            rcgen::KeyIdMethod::PreSpecified(Sha256::digest(key.der_bytes()).to_vec());
    }
    params
}
fn identity(name: &str, names: Vec<String>, issuer: &Issuer<'_, key::KeyPair>) -> Identity {
    let key = key::generate();
    let mut params = parameters(name, names, &key);
    params.is_ca = IsCa::ExplicitNoCa;
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let cert = params.signed_by(&key, issuer).unwrap();
    Identity {
        cert: cert.der().to_vec(),
        key: key.serialize_der(),
    }
}
pub fn acceptor(wrong_name: bool) -> tokio_rustls::TlsAcceptor {
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
    use std::sync::Arc;
    let identity = if wrong_name {
        &fixtures().wrong
    } else {
        &fixtures().good
    };
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from(identity.cert.clone())],
        PrivatePkcs8KeyDer::from(identity.key.clone()).into(),
    )
    .unwrap();
    tokio_rustls::TlsAcceptor::from(Arc::new(config))
}

#[test]
fn server_certificates_have_short_current_validity() {
    use x509_parser::prelude::{FromDer as _, X509Certificate};
    let fixtures = fixtures();
    let now = OffsetDateTime::now_utc();
    for identity in [&fixtures.good, &fixtures.wrong] {
        let leaf = X509Certificate::from_der(&identity.cert).unwrap().1;
        let validity = leaf.validity();
        let start = validity.not_before.to_datetime();
        let end = validity.not_after.to_datetime();
        assert!(
            end - start <= Duration::days(398),
            "server certificate validity exceeds 398 days: {start} to {end}"
        );
        assert!(start <= now - Duration::hours(23));
        assert!(start >= now - Duration::hours(25));
        assert!(end > now + Duration::days(90));
        assert!(!leaf.is_ca());
        assert!(
            leaf.extended_key_usage()
                .unwrap()
                .unwrap()
                .value
                .server_auth
        );
        let ca = X509Certificate::from_der(&fixtures.ca).unwrap().1;
        assert_eq!(leaf.issuer(), ca.subject());
    }
}
