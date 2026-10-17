//! CA snapshots prepared before native consent. Settings Save carries this immutable candidate to sealing.
//! Depends on the certificate validators and SHA-256; unchanged authenticated snapshots never touch disk.

use std::io;

use sha2::{Digest, Sha256};

use super::{NetworkSettings, certificate_files, check};

/// A complete candidate whose certificate bytes are fixed before confirmation.
#[derive(Clone)]
pub struct PreparedSettings {
    settings: NetworkSettings,
    pub(super) payload: String,
}

impl std::fmt::Debug for PreparedSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedSettings")
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl PreparedSettings {
    /// The exact candidate shown for confirmation and later authenticated; no mutable access is exposed.
    #[must_use]
    pub fn settings(&self) -> &NetworkSettings {
        &self.settings
    }

    /// The short fingerprint of a CA file or the complete folder snapshot, for the native dialog.
    /// Folder members are sorted and length-prefixed so boundaries and directory enumeration cannot alias.
    #[must_use]
    pub fn certificate_fingerprint(&self, key: &str) -> Option<String> {
        let mut digest = Sha256::new();
        match key {
            "SSL_CERT_FILE" => digest.update(self.settings.file_certificates.as_ref()?),
            "SSL_CERT_DIR" => {
                let mut files: Vec<_> = self.settings.dir_certificates.as_ref()?.iter().collect();
                files.sort();
                for bytes in files {
                    digest.update((bytes.len() as u64).to_be_bytes());
                    digest.update(bytes);
                }
            }
            _ => return None,
        }
        let hex = format!("{:x}", digest.finalize());
        Some([&hex[0..4], &hex[4..8], &hex[8..12], &hex[12..16]].join(" "))
    }
}

impl NetworkSettings {
    /// Validates and retains a CA snapshot unless this field already has authenticated bytes.
    ///
    /// # Errors
    /// A plain problem about the certificate file or folder; the caller supplies its field label.
    pub fn snapshot_certificate(&mut self, key: &str) -> Result<(), String> {
        match key {
            "SSL_CERT_FILE" if self.file_certificates.is_none() => {
                if let Some(path) = &self.ssl_cert_file {
                    check::check_path_text("Certificate file", &path.to_string_lossy())?;
                    let bytes = check::read_capped(path)?;
                    check::parse_certificates(&bytes)?;
                    self.file_certificates = Some(bytes);
                }
            }
            "SSL_CERT_DIR" if self.dir_certificates.is_none() => {
                if let Some(path) = &self.ssl_cert_dir {
                    check::check_certificate_dir("Certificate folder", &path.to_string_lossy())?;
                    self.dir_certificates = Some(certificate_files(path)?);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// The saved file's certificate count, derived only from its authenticated bytes.
    #[must_use]
    pub fn saved_certificate_count(&self) -> Option<usize> {
        self.file_certificates
            .as_deref()
            .and_then(|bytes| check::parse_certificates(bytes).ok())
            .map(|certificates| certificates.len())
    }
}

/// Prepares the complete candidate before native confirmation, reusing existing authenticated CA bytes.
///
/// # Errors
/// A changed certificate file or folder cannot be read or parsed.
pub fn prepare_settings(settings: &NetworkSettings) -> io::Result<PreparedSettings> {
    let mut settings = settings.clone();
    for key in ["SSL_CERT_FILE", "SSL_CERT_DIR"] {
        settings
            .snapshot_certificate(key)
            .map_err(io::Error::other)?;
    }
    let payload = super::record::payload_text(&settings)?;
    Ok(PreparedSettings { settings, payload })
}
