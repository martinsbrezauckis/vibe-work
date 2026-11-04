//! Fresh, short-lived proxy identities and acceptors using each target's product TLS stack.
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
    #[cfg(target_os = "linux")]
    key: Vec<u8>,
    #[cfg(not(target_os = "linux"))]
    pkcs12: Vec<u8>,
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
                ca.der(),
            ),
            wrong: identity(
                "wrong-name.example",
                vec!["wrong-name.example".into()],
                &issuer,
                ca.der(),
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
fn identity(
    name: &str,
    names: Vec<String>,
    issuer: &Issuer<'_, key::KeyPair>,
    ca: &[u8],
) -> Identity {
    #[cfg(target_os = "linux")]
    let _ = ca; // The rustls server sends the leaf; the client already has its root.
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
        #[cfg(not(target_os = "linux"))]
        pkcs12: pkcs12(name, cert.der(), &key.serialize_der(), ca),
        cert: cert.der().to_vec(),
        #[cfg(target_os = "linux")]
        key: key.serialize_der(),
    }
}
#[cfg(not(target_os = "linux"))]
fn pkcs12(name: &str, cert: &[u8], key: &[u8], ca: &[u8]) -> Vec<u8> {
    use p12_keystore::{
        Certificate, EncryptionAlgorithm, KeyStore, KeyStoreEntry, MacAlgorithm, PrivateKey,
        PrivateKeyChain,
    };
    let chain = PrivateKeyChain::new(
        name,
        PrivateKey::from_der(key).unwrap(),
        [
            Certificate::from_der(cert).unwrap(),
            Certificate::from_der(ca).unwrap(),
        ],
    );
    let mut store = KeyStore::new();
    store.add_entry(name, KeyStoreEntry::PrivateKeyChain(chain));
    // Native importers need the compatible PKCS#12 envelope, rather than the writer's PBES2 default.
    // This protects only throwaway test keys; the certificates themselves are signed with SHA-256.
    store
        .writer("test-fixture")
        .encryption_algorithm(EncryptionAlgorithm::PbeWithShaAnd3KeyTripleDesCbc)
        .mac_algorithm(MacAlgorithm::HmacSha1)
        .write()
        .unwrap()
}
#[cfg(target_os = "linux")]
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
#[cfg(not(target_os = "linux"))]
pub fn acceptor(wrong_name: bool) -> tokio_native_tls::TlsAcceptor {
    let identity = if wrong_name {
        &fixtures().wrong
    } else {
        &fixtures().good
    };
    let identity = native_tls::Identity::from_pkcs12(&identity.pkcs12, "test-fixture").unwrap();
    tokio_native_tls::TlsAcceptor::from(native_tls::TlsAcceptor::new(identity).unwrap())
}

#[test]
fn server_certificates_have_short_current_validity() {
    use x509_parser::prelude::{FromDer as _, X509Certificate};
    let fixtures = fixtures();
    let now = OffsetDateTime::now_utc();
    for identity in [&fixtures.good, &fixtures.wrong] {
        // Inspect exactly what rustls receives, or what the native stack imports from PKCS#12.
        #[cfg(not(target_os = "linux"))]
        {
            // Encoded OIDs for SHA-1/3DES PBE and SHA-1 MAC, as required by native importers.
            for oid in [
                &[6, 10, 42, 134, 72, 134, 247, 13, 1, 12, 1, 3][..],
                &[6, 5, 43, 14, 3, 2, 26][..],
            ] {
                assert!(
                    identity.pkcs12.windows(oid.len()).any(|bytes| bytes == oid),
                    "PKCS#12 lacks a native-compatible envelope OID: {oid:?}"
                );
            }
            let bundle = p12_keystore::KeyStore::from_pkcs12(
                &identity.pkcs12,
                "test-fixture",
                p12_keystore::Pkcs12ImportPolicy::Strict,
            )
            .unwrap();
            let cert = bundle
                .entries()
                .find_map(|(_, entry)| match entry {
                    p12_keystore::KeyStoreEntry::PrivateKeyChain(chain) => {
                        Some(chain.certs()[0].as_der())
                    }
                    _ => None,
                })
                .unwrap();
            assert_eq!(cert, identity.cert);
        }
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
