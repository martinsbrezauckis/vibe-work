//! Test-only signing keys: Linux reuses ring, other targets use pure Rust without a cross C SDK.
//! Called by the proxy identity generator; every private key is throwaway and stays in memory.
#[cfg(not(target_os = "linux"))]
pub use native::KeyPair;
#[cfg(target_os = "linux")]
pub use rcgen::KeyPair;

pub fn generate() -> KeyPair {
    #[cfg(target_os = "linux")]
    {
        KeyPair::generate().unwrap()
    }
    #[cfg(not(target_os = "linux"))]
    {
        native::generate()
    }
}

#[cfg(not(target_os = "linux"))]
mod native {
    use rcgen::{PublicKeyData, SignatureAlgorithm, SigningKey};
    use rsa::sha2::{Digest as _, Sha256};
    use rsa::{RsaPrivateKey, pkcs1::EncodeRsaPublicKey as _, pkcs8::EncodePrivateKey as _};

    pub struct KeyPair {
        private: RsaPrivateKey,
        public: Vec<u8>,
    }
    pub fn generate() -> KeyPair {
        let private = RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
        let public = private
            .to_public_key()
            .to_pkcs1_der()
            .unwrap()
            .as_bytes()
            .to_vec();
        KeyPair { private, public }
    }
    impl KeyPair {
        pub fn serialize_der(&self) -> Vec<u8> {
            self.private.to_pkcs8_der().unwrap().as_bytes().to_vec()
        }
    }
    impl PublicKeyData for KeyPair {
        fn der_bytes(&self) -> &[u8] {
            &self.public
        }
        fn algorithm(&self) -> &'static SignatureAlgorithm {
            &rcgen::PKCS_RSA_SHA256
        }
    }
    impl SigningKey for KeyPair {
        fn sign(&self, message: &[u8]) -> Result<Vec<u8>, rcgen::Error> {
            self.private
                .sign(rsa::Pkcs1v15Sign::new::<Sha256>(), &Sha256::digest(message))
                .map_err(|_| rcgen::Error::RemoteKeyError)
        }
    }
}
