//! The proxy and certificate settings every HTTPS client of the core uses: the six values Vibe knows (`HTTP_PROXY`,
//! `HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY`, `SSL_CERT_FILE`, `SSL_CERT_DIR`; Vibe's `/proxy-setup`, protocol
//! `config/proxy/read|write`).
//!
//! Vibe's model traffic does not leave through Vibe: its provider address is the local model proxy
//! (vw-keyproxy), which forwards to Mistral with this crate's client. So a proxy saved for Vibe alone would
//! not move one model request. When the user saves in Settings, the core writes the values to Vibe (its `.env`, for
//! Vibe's own traffic) **and to a file of its own** in the app's data folder ([`settings_file`]), and reads only that
//! file at its start.
//!
//! **Trust boundary:** the core never reads Vibe's `.env`. The agent can write that file, and the core's clients carry
//! the real Mistral key (the model proxy adds it), so a proxy and certificate authority planted there would let
//! an agent read the key at the next launch. Vibe holds only the placeholder key, so its own use of the `.env` is
//! harmless. The app-data file is agent-writable too: custom values are authenticated with a random key in the
//! Mistral key's vault, rotated on every persisted Save after native confirmation. Saved CA bytes are authenticated
//! snapshots fixed before confirmation; replacing the protected key also revokes all older authenticated records.
//!
//! Rules, copied from Vibe (`vibe/utils/http.py`, `vibe/core/config/vibe_schema.py`) so both sides agree:
//! - A value in the process environment (either case) wins over the same key in the file; the file fills the gaps.
//! - The proxy for a request is picked by its scheme (`HTTP_PROXY` for http, `HTTPS_PROXY` for https), then
//!   `ALL_PROXY`; with none of them set the request goes direct (here: with the system's own proxy settings, as
//!   before this module existed).
//! - A certificate file or folder is added to the trusted certificates; it never replaces them.
//!
//! Depends on: reqwest, `serde_json`, tracing, std. Called by: `client` (every client it builds), vw-core-api
//! (`network`: read, test, save), the app (`startup`: loads the saved settings before the first client exists), vw-gate
//! (`confirm_text`: masking).
//! Invariants:
//! - Settings are applied at the start of the process (and by the Test button, to a throwaway client). A client
//!   that already exists keeps what it was built with, so a saved change takes effect at the next start.
//! - A value that cannot be applied (an unreadable certificate, an address that no longer parses) is logged
//!   without its content and skipped; it never stops a client from being built.
//! - Nothing here logs a proxy address unmasked.

mod check;
mod prepare;
mod probe;
mod record;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};

pub use check::{
    CertCheck, LOOPBACK, MAX_CERT_BYTES, MAX_VALUE_LEN, check_certificate_dir,
    check_certificate_file, check_no_proxy, check_path_text, check_proxy_url, form_proxy_url,
    has_sign_in, mask_proxy_url, scrub, with_loopback, without_loopback,
};
pub use prepare::{PreparedSettings, prepare_settings};
pub use probe::{Probe, ProbeKind, probe};
pub use record::{LoadedSettings, NetworkMacKey, read_settings, seal_settings};

/// Vibe's six keys, in the order its `/proxy-setup` lists them.
pub const KEYS: [&str; 6] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
];

/// The core's settings file is read up to this many bytes (a larger file is not a settings file).
const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// A certificate folder is read up to this many files.
const MAX_DIR_FILES: usize = 1000;

/// One set of proxy and certificate values. Empty means "not set".
#[derive(Clone, Default, PartialEq, Eq)]
pub struct NetworkSettings {
    /// `HTTP_PROXY`: the proxy for http requests.
    pub http_proxy: Option<String>,
    /// `HTTPS_PROXY`: the proxy for https requests.
    pub https_proxy: Option<String>,
    /// `ALL_PROXY`: the proxy for any request the two above do not cover.
    pub all_proxy: Option<String>,
    /// `NO_PROXY`: comma-separated hosts, domains, addresses and ranges that bypass the proxy.
    pub no_proxy: Option<String>,
    /// `SSL_CERT_FILE`: a PEM file of extra trusted certificates.
    pub ssl_cert_file: Option<PathBuf>,
    /// `SSL_CERT_DIR`: a folder of extra trusted certificates.
    pub ssl_cert_dir: Option<PathBuf>,
    // Only verified records supply these snapshots; launch never rereads saved CA paths.
    file_certificates: Option<Vec<u8>>,
    dir_certificates: Option<Vec<Vec<u8>>>,
}

impl fmt::Debug for NetworkSettings {
    /// The addresses with their sign-in hidden: a `Debug` of the settings is safe to log.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let masked = |value: &Option<String>| value.as_deref().map(mask_proxy_url);
        f.debug_struct("NetworkSettings")
            .field("http_proxy", &masked(&self.http_proxy))
            .field("https_proxy", &masked(&self.https_proxy))
            .field("all_proxy", &masked(&self.all_proxy))
            .field("no_proxy", &self.no_proxy)
            .field("ssl_cert_file", &self.ssl_cert_file)
            .field("ssl_cert_dir", &self.ssl_cert_dir)
            .finish_non_exhaustive()
    }
}

/// Which requests a proxy address serves.
#[derive(Clone, Copy)]
enum Covers {
    Http,
    Https,
    All,
}

impl Covers {
    fn proxy(self, address: &str) -> reqwest::Result<reqwest::Proxy> {
        match self {
            Self::Http => reqwest::Proxy::http(address),
            Self::Https => reqwest::Proxy::https(address),
            Self::All => reqwest::Proxy::all(address),
        }
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

impl NetworkSettings {
    /// The settings `lookup` answers for each of [`KEYS`]. A key it does not know, or knows as empty, is unset.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            http_proxy: non_empty(lookup("HTTP_PROXY")),
            https_proxy: non_empty(lookup("HTTPS_PROXY")),
            all_proxy: non_empty(lookup("ALL_PROXY")),
            no_proxy: non_empty(lookup("NO_PROXY")),
            ssl_cert_file: non_empty(lookup("SSL_CERT_FILE")).map(PathBuf::from),
            ssl_cert_dir: non_empty(lookup("SSL_CERT_DIR")).map(PathBuf::from),
            ..Self::default()
        }
    }

    /// The values as JSON. Custom values require [`seal_settings`] before persistence; this text alone is untrusted.
    #[must_use]
    pub fn to_file_text(&self) -> String {
        let values: serde_json::Map<String, serde_json::Value> = KEYS
            .iter()
            .filter_map(|key| self.get(key).map(|value| ((*key).to_owned(), value.into())))
            .collect();
        serde_json::to_string_pretty(&values).unwrap_or_else(|_| "{}".to_owned())
    }

    /// The no-proxy list Vibe's child process should get from the app instead of the one in its `.env`, when the app
    /// itself runs with a proxy in its environment: Vibe lets a non-empty `NO_PROXY` of its process environment win
    /// over its `.env`, so the list saved in Settings (in `self`, the effective settings) must travel in the
    /// environment too. `None` when the environment has no proxy or no list is saved.
    #[must_use]
    pub fn child_no_proxy(&self, environment: &Self) -> Option<String> {
        environment
            .has_proxy()
            .then(|| self.no_proxy.clone())
            .flatten()
    }

    /// The settings this process's environment defines. For the proxy keys the lower-case spelling wins over the
    /// upper-case one, as in Python's `getproxies`, which Vibe uses.
    #[must_use]
    pub fn from_environment() -> Self {
        Self::from_lookup(|key| {
            let get = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
            match key {
                "SSL_CERT_FILE" | "SSL_CERT_DIR" => get(key),
                _ => get(&key.to_ascii_lowercase()).or_else(|| get(key)),
            }
        })
    }

    /// What applies when the file holds `saved` and the process environment holds `environment`: per key, the
    /// environment first.
    #[must_use]
    pub fn effective(saved: &Self, environment: &Self) -> Self {
        Self {
            http_proxy: environment
                .http_proxy
                .clone()
                .or_else(|| saved.http_proxy.clone()),
            https_proxy: environment
                .https_proxy
                .clone()
                .or_else(|| saved.https_proxy.clone()),
            all_proxy: environment
                .all_proxy
                .clone()
                .or_else(|| saved.all_proxy.clone()),
            no_proxy: environment
                .no_proxy
                .clone()
                .or_else(|| saved.no_proxy.clone()),
            ssl_cert_file: environment
                .ssl_cert_file
                .clone()
                .or_else(|| saved.ssl_cert_file.clone()),
            ssl_cert_dir: environment
                .ssl_cert_dir
                .clone()
                .or_else(|| saved.ssl_cert_dir.clone()),
            file_certificates: environment
                .ssl_cert_file
                .is_none()
                .then(|| saved.file_certificates.clone())
                .flatten(),
            dir_certificates: environment
                .ssl_cert_dir
                .is_none()
                .then(|| saved.dir_certificates.clone())
                .flatten(),
        }
    }

    /// The value of one of [`KEYS`] as text (`None` when unset).
    #[must_use]
    pub fn get(&self, key: &str) -> Option<String> {
        match key {
            "HTTP_PROXY" => self.http_proxy.clone(),
            "HTTPS_PROXY" => self.https_proxy.clone(),
            "ALL_PROXY" => self.all_proxy.clone(),
            "NO_PROXY" => self.no_proxy.clone(),
            "SSL_CERT_FILE" => self
                .ssl_cert_file
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            "SSL_CERT_DIR" => self
                .ssl_cert_dir
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            _ => None,
        }
    }

    /// True when no key is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        KEYS.iter().all(|key| self.get(key).is_none())
    }

    /// True when at least one proxy address is set.
    #[must_use]
    pub fn has_proxy(&self) -> bool {
        self.http_proxy.is_some() || self.https_proxy.is_some() || self.all_proxy.is_some()
    }

    /// True when proxy routing or extra trust anchors require authenticated Save provenance.
    #[must_use]
    pub fn has_custom(&self) -> bool {
        self.has_proxy() || self.ssl_cert_file.is_some() || self.ssl_cert_dir.is_some()
    }

    /// The same settings with a different value for one key (`None` clears it).
    #[must_use]
    pub fn with(mut self, key: &str, value: Option<String>) -> Self {
        let value = non_empty(value);
        if self.get(key) == value {
            return self;
        }
        match key {
            "HTTP_PROXY" => self.http_proxy = value,
            "HTTPS_PROXY" => self.https_proxy = value,
            "ALL_PROXY" => self.all_proxy = value,
            "NO_PROXY" => self.no_proxy = value,
            "SSL_CERT_FILE" => {
                self.ssl_cert_file = value.map(PathBuf::from);
                self.file_certificates = None;
            }
            "SSL_CERT_DIR" => {
                self.ssl_cert_dir = value.map(PathBuf::from);
                self.dir_certificates = None;
            }
            _ => {}
        }
        self
    }

    /// `builder` with these settings: the proxies (each with the no-proxy list and the addresses of this computer
    /// on it) and the extra trusted certificates. With no proxy set the builder keeps reqwest's own proxy
    /// handling (the environment and the system's settings).
    pub fn apply(&self, mut builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
        if self.has_proxy() {
            let bypass = reqwest::NoProxy::from_string(&with_loopback(
                self.no_proxy.as_deref().unwrap_or_default(),
            ));
            // The scheme-specific proxies come first, `ALL_PROXY` last: reqwest uses the first that matches.
            for (key, covers, address) in [
                ("HTTP_PROXY", Covers::Http, &self.http_proxy),
                ("HTTPS_PROXY", Covers::Https, &self.https_proxy),
                ("ALL_PROXY", Covers::All, &self.all_proxy),
            ] {
                let Some(address) = address else { continue };
                if let Ok(proxy) = covers.proxy(address) {
                    builder = builder.proxy(proxy.no_proxy(bypass.clone()));
                } else {
                    tracing::warn!(
                        key,
                        address = %mask_proxy_url(address),
                        "network settings: this proxy address is not usable and is skipped"
                    );
                }
            }
        }
        let certificates = self.certificates();
        if certificates.is_empty() {
            builder
        } else {
            builder.tls_certs_merge(certificates)
        }
    }

    /// Verified saved snapshots, or files for trusted environment values and key-free Test candidates.
    /// What cannot be read or parsed is logged and skipped.
    fn certificates(&self) -> Vec<reqwest::Certificate> {
        let mut found = Vec::new();
        if let Some(file) = &self.ssl_cert_file {
            let bytes = self
                .file_certificates
                .clone()
                .map_or_else(|| check::read_capped(file), Ok);
            match bytes.and_then(|bytes| check::parse_certificates(&bytes)) {
                Ok(certs) => found.extend(certs),
                Err(problem) => tracing::warn!(
                    file = %file.display(),
                    %problem,
                    "network settings: the certificate file is not used"
                ),
            }
        }
        if let Some(dir) = &self.ssl_cert_dir {
            match self
                .dir_certificates
                .clone()
                .map_or_else(|| certificate_files(dir), Ok)
            {
                Ok(files) => found.extend(
                    files
                        .iter()
                        .filter_map(|bytes| check::parse_certificates(bytes).ok())
                        .flatten(),
                ),
                Err(_) => {
                    tracing::warn!(dir = %dir.display(), "network settings: the certificate folder cannot be read");
                }
            }
        }
        found
    }
}

/// The certificates of the regular files in `dir` (a certificate folder holds PEM files, often under hash names);
/// a file that is no certificate file is skipped.
fn certificate_files(dir: &Path) -> Result<Vec<Vec<u8>>, String> {
    let entries =
        std::fs::read_dir(dir).map_err(|_| "The certificate folder cannot be read.".to_owned())?;
    let mut found = Vec::new();
    for entry in entries.flatten().take(MAX_DIR_FILES) {
        let path = entry.path();
        let Ok(bytes) = check::read_capped(&path) else {
            continue;
        };
        if check::parse_certificates(&bytes).is_ok() {
            found.push(bytes);
        }
    }
    Ok(found)
}

static CURRENT: RwLock<Option<Arc<NetworkSettings>>> = RwLock::new(None);

/// Sets the settings every client built from now on uses. Called once at the start of the process.
pub fn set_current(settings: NetworkSettings) {
    *CURRENT.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(settings));
}

/// The settings clients are built with (empty until [`set_current`] ran).
#[must_use]
pub fn current() -> Arc<NetworkSettings> {
    CURRENT
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .unwrap_or_default()
}

/// Where the core keeps the settings saved in Settings > Network: a file of its own in the app's data folder, next to
/// its other settings (the core writes it user-only, and never reads Vibe's `.env`; see the module docs).
#[must_use]
pub fn settings_file(app_data: &Path) -> PathBuf {
    app_data.join("network").join("settings.json")
}

/// Verifies the core's settings record before applying it, lets the trusted process environment win per key,
/// and makes the result current before any key-bearing client is constructed.
pub fn load_for_launch(file: &Path, key: Option<&NetworkMacKey>) -> LoadedSettings {
    let mut loaded = read_settings(file, key);
    let effective =
        NetworkSettings::effective(&loaded.settings, &NetworkSettings::from_environment());
    tracing::info!(settings = ?effective, "network settings loaded");
    set_current(effective.clone());
    loaded.settings = effective;
    loaded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(pairs: &'static [(&'static str, &'static str)]) -> NetworkSettings {
        NetworkSettings::from_lookup(|key| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| (*v).to_owned())
        })
    }

    #[test]
    fn the_environment_wins_per_key_and_the_file_fills_the_gaps() {
        let saved = lookup(&[
            ("HTTPS_PROXY", "http://saved:1"),
            ("NO_PROXY", "saved.corp"),
        ]);
        let environment = lookup(&[("HTTPS_PROXY", "http://env:2")]);
        let effective = NetworkSettings::effective(&saved, &environment);
        assert_eq!(effective.https_proxy.as_deref(), Some("http://env:2"));
        assert_eq!(effective.no_proxy.as_deref(), Some("saved.corp"));
        assert!(effective.http_proxy.is_none());
    }

    #[test]
    fn every_key_follows_the_environment_first_rule() {
        let saved = lookup(&[
            ("HTTP_PROXY", "http://saved:1"),
            ("HTTPS_PROXY", "http://saved:1"),
            ("ALL_PROXY", "http://saved:1"),
            ("NO_PROXY", "saved.corp"),
            ("SSL_CERT_FILE", "/saved.pem"),
            ("SSL_CERT_DIR", "/saved"),
        ]);
        let environment = lookup(&[
            ("HTTP_PROXY", "http://env:2"),
            ("HTTPS_PROXY", "http://env:2"),
            ("ALL_PROXY", "http://env:2"),
            ("NO_PROXY", "env.corp"),
            ("SSL_CERT_FILE", "/env.pem"),
            ("SSL_CERT_DIR", "/env"),
        ]);
        let effective = NetworkSettings::effective(&saved, &environment);
        for key in KEYS {
            assert_eq!(effective.get(key), environment.get(key), "{key}");
        }
        // And with the environment silent, the file's values are what applies, key by key.
        let quiet = NetworkSettings::effective(&saved, &NetworkSettings::default());
        for key in KEYS {
            assert_eq!(quiet.get(key), saved.get(key), "{key}");
        }
    }

    #[test]
    fn debug_never_shows_a_sign_in() {
        let settings = lookup(&[("HTTPS_PROXY", "http://user:hunter2@proxy:8080")]);
        let shown = format!("{settings:?}");
        assert!(
            !shown.contains("hunter2") && !shown.contains("user:"),
            "{shown}"
        );
        assert!(shown.contains("http://proxy:8080"));
    }

    #[test]
    fn blank_values_are_unset_and_with_replaces_one_key() {
        let settings = lookup(&[("HTTP_PROXY", "   "), ("SSL_CERT_FILE", "/ca.pem")]);
        assert!(settings.http_proxy.is_none());
        assert!(!settings.is_empty());
        let cleared = settings.with("SSL_CERT_FILE", Some(String::new()));
        assert!(cleared.is_empty());
        assert!(!cleared.has_proxy());
    }

    #[test]
    fn the_core_file_round_trips_and_ignores_what_is_not_its_own() {
        let dir = std::env::temp_dir().join(format!("vw-network-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("settings.json");
        let key = NetworkMacKey::new(zeroize::Zeroizing::new([7; 32]));
        assert!(
            read_settings(&file, Some(&key)).settings.is_empty(),
            "a missing file is empty"
        );
        let settings = lookup(&[
            ("HTTPS_PROXY", "http://alice:s3cret@proxy.corp:8080"),
            ("NO_PROXY", ".corp,localhost,127.0.0.1,::1"),
        ]);
        let prepared = prepare_settings(&settings).expect("prepared");
        assert!(!format!("{prepared:?}").contains("s3cret"));
        std::fs::write(&file, seal_settings(&prepared, Some(&key)).expect("sealed"))
            .expect("write");
        assert_eq!(read_settings(&file, Some(&key)).settings, settings);
        // Other keys are not read; a file that is not the object (or has a non-text value) is ignored whole.
        std::fs::write(
            &file,
            r#"{"HTTPS_PROXY": "http://p:1", "MISTRAL_API_KEY": "nope"}"#,
        )
        .expect("write");
        let read = read_settings(&file, Some(&key)).settings;
        assert!(read.is_empty(), "unsigned custom settings are ignored");
        assert_eq!(read.get("MISTRAL_API_KEY"), None);
        for bad in ["not json", "[1,2]", r#"{"HTTPS_PROXY": 5}"#] {
            std::fs::write(&file, bad).expect("write");
            assert!(
                read_settings(&file, Some(&key)).settings.is_empty(),
                "{bad}"
            );
        }
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn the_saved_list_travels_to_vibe_only_when_the_app_runs_with_a_proxy() {
        let saved = lookup(&[("NO_PROXY", ".corp,localhost,127.0.0.1,::1")]);
        let with_proxy = lookup(&[("HTTPS_PROXY", "http://env:3128")]);
        assert_eq!(
            saved.child_no_proxy(&with_proxy).as_deref(),
            Some(".corp,localhost,127.0.0.1,::1")
        );
        assert_eq!(saved.child_no_proxy(&NetworkSettings::default()), None);
        assert_eq!(NetworkSettings::default().child_no_proxy(&with_proxy), None);
    }

    #[test]
    fn an_unusable_address_or_certificate_never_stops_a_client_from_being_built() {
        let broken = NetworkSettings {
            https_proxy: Some("not a url".to_owned()),
            ssl_cert_file: Some(PathBuf::from("/definitely/not/here.pem")),
            ..NetworkSettings::default()
        };
        assert!(broken.apply(crate::tls_builder()).build().is_ok());
    }
}
