//! Opt-in guard settings and trusted CA snapshots. Depends on the signed network settings; called at launch.
//! Settings precede process/system proxies here; the default clients keep their established precedence.
use super::{NetworkSettings, certificate_files, check};
use std::sync::{Arc, PoisonError, RwLock};
static GUARD: RwLock<Option<Arc<NetworkSettings>>> = RwLock::new(None);

/// Settings for the guard, before system proxy discovery.
#[must_use]
pub fn guard_settings(saved: &NetworkSettings, environment: &NetworkSettings) -> NetworkSettings {
    let mut result = environment.clone();
    if saved.has_proxy() {
        result.http_proxy.clone_from(&saved.http_proxy);
        result.https_proxy.clone_from(&saved.https_proxy);
        result.all_proxy.clone_from(&saved.all_proxy);
        result.no_proxy.clone_from(&saved.no_proxy);
    } else if saved.no_proxy.is_some() {
        result.no_proxy.clone_from(&saved.no_proxy);
    }
    if saved.ssl_cert_file.is_some() {
        result.ssl_cert_file.clone_from(&saved.ssl_cert_file);
        result
            .file_certificates
            .clone_from(&saved.file_certificates);
    }
    if saved.ssl_cert_dir.is_some() {
        result.ssl_cert_dir.clone_from(&saved.ssl_cert_dir);
        result.dir_certificates.clone_from(&saved.dir_certificates);
    }
    result
}
/// The launch snapshot for the guard (falls back to current for embedded/test users).
#[must_use]
pub fn guard_current() -> Arc<NetworkSettings> {
    GUARD
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .unwrap_or_else(super::current)
}
pub(super) fn set(saved: &NetworkSettings, environment: &NetworkSettings) {
    *GUARD.write().unwrap_or_else(PoisonError::into_inner) =
        Some(Arc::new(guard_settings(saved, environment)));
}
impl NetworkSettings {
    /// Trusted DER roots for proxy TLS: authenticated saved bytes, or trusted environment candidates.
    /// Never rereads a saved path when an authenticated snapshot exists.
    #[must_use]
    pub fn guard_certificates(&self) -> Vec<Vec<u8>> {
        use rustls_pki_types::{CertificateDer, pem::PemObject as _};
        let mut bundles = Vec::new();
        if let Some(file) = &self.ssl_cert_file
            && let Ok(bytes) = self
                .file_certificates
                .clone()
                .map_or_else(|| check::read_capped(file), Ok)
        {
            bundles.push(bytes);
        }
        if let Some(dir) = &self.ssl_cert_dir
            && let Ok(bytes) = self
                .dir_certificates
                .clone()
                .map_or_else(|| certificate_files(dir), Ok)
        {
            bundles.extend(bytes);
        }
        bundles
            .iter()
            .flat_map(|bytes| {
                CertificateDer::pem_slice_iter(bytes)
                    .filter_map(Result::ok)
                    .map(|c| c.to_vec())
            })
            .collect()
    }
}
