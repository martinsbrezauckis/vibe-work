//! Checks and masking for the proxy and certificate settings: what may be saved, in plain words, and how an
//! address that carries a sign-in is shown.
//!
//! Depends on: reqwest (`Url`, `Certificate`), std. Called by: `network` (re-exported), vw-core-api
//! (`network`), vw-gate (`confirm_text`: the masked address in the confirmation dialog).
//! Invariants:
//! - A problem text never quotes the value it is about (a proxy address can carry `user:password@`).
//! - An address is shown only from its PARSED parts: [`mask_proxy_url`] (dialogs, logs) gives `scheme://host:port` and
//!   nothing else, so what is shown is the host the client really connects to (a hand-made `#@` or `/@` in the text
//!   cannot make another host look like the proxy); [`form_proxy_url`] (the Settings field) adds `***@` when there is a
//!   sign-in. An address that does not parse is not shown at all.
//! - The rules also keep Vibe's own `.env` writer from damaging a value: a control character, `${` (python-dotenv
//!   would replace it with an environment variable) and a doubled backslash (python-dotenv collapses it, which
//!   breaks a network path) are refused.

use std::path::Path;

/// The longest value accepted for any one setting.
pub const MAX_VALUE_LEN: usize = 2000;
/// The largest certificate file read.
pub const MAX_CERT_BYTES: u64 = 8 * 1024 * 1024;
/// Addresses of this computer. Vibe sends its model requests to the model proxy on 127.0.0.1, so they must
/// never go through a company proxy; they are always part of the no-proxy list.
pub const LOOPBACK: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// What reading a certificate file found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CertCheck {
    /// How many certificates the file holds.
    pub certificates: usize,
}

/// What is shown instead of an address that does not parse.
const NOT_SHOWN: &str = "(address not shown)";

/// The parts of a proxy address that may be shown.
struct Shown {
    scheme: String,
    host: String,
    port: Option<u16>,
    sign_in: bool,
}

impl Shown {
    fn of(text: &str) -> Option<Self> {
        let text = text.trim();
        // The environment often holds `host:port` without a scheme; reqwest reads that as http.
        let candidate = if text.contains("://") {
            text.to_owned()
        } else {
            format!("http://{text}")
        };
        let url = reqwest::Url::parse(&candidate).ok()?;
        let host = url.host_str().filter(|host| !host.is_empty())?.to_owned();
        Some(Self {
            scheme: url.scheme().to_owned(),
            host,
            port: url.port_or_known_default(),
            sign_in: !url.username().is_empty() || url.password().is_some(),
        })
    }

    fn show(&self, mark_sign_in: bool) -> String {
        let port = self.port.map(|port| format!(":{port}")).unwrap_or_default();
        let sign_in = if mark_sign_in && self.sign_in {
            "***@"
        } else {
            ""
        };
        format!("{}://{sign_in}{}{port}", self.scheme, self.host)
    }
}

/// `scheme://host:port` of a proxy address, read from the parsed address: no sign-in, path, query or fragment, so a
/// dialog or a log line can never show more than the host the client connects to. `http://user:secret@proxy:8080`
/// becomes `http://proxy:8080`. An address that does not parse becomes a fixed placeholder.
#[must_use]
pub fn mask_proxy_url(text: &str) -> String {
    Shown::of(text).map_or_else(|| NOT_SHOWN.to_owned(), |shown| shown.show(false))
}

/// The same for the Settings field: `http://***@proxy:8080` when the address has a sign-in (so a field that still
/// shows `***@` is told apart from one the user retyped without it).
#[must_use]
pub fn form_proxy_url(text: &str) -> String {
    Shown::of(text).map_or_else(|| NOT_SHOWN.to_owned(), |shown| shown.show(true))
}

/// Whether `text` carries a sign-in (a user name or password before the host). Text that does not parse counts as
/// carrying one when it holds an `@`.
#[must_use]
pub fn has_sign_in(text: &str) -> bool {
    Shown::of(text).map_or_else(|| text.contains('@'), |shown| shown.sign_in)
}

/// `text` with every given raw value replaced by its masked form. For error texts that came from outside and
/// may repeat a value that was sent.
#[must_use]
pub fn scrub(text: &str, raw_values: &[&str]) -> String {
    let mut out = text.to_owned();
    for raw in raw_values.iter().filter(|raw| !raw.is_empty()) {
        out = out.replace(raw, &mask_proxy_url(raw));
    }
    out
}

fn plain_text_problem(value: &str) -> Option<&'static str> {
    if value.chars().any(char::is_control) {
        return Some("It cannot contain line breaks or other control characters.");
    }
    if value.contains("${") {
        return Some("It cannot contain the characters ${ .");
    }
    if value.chars().count() > MAX_VALUE_LEN {
        return Some("It is too long.");
    }
    None
}

/// Checks one proxy address (`label` names the setting in the problem text) and returns it trimmed with a
/// lower-case scheme.
///
/// # Errors
/// A plain-words problem, without the address in it.
pub fn check_proxy_url(label: &str, raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if let Some(problem) = plain_text_problem(value) {
        return Err(format!("{label}: {problem}"));
    }
    if value.chars().any(char::is_whitespace) {
        return Err(format!("{label}: The address cannot contain spaces."));
    }
    // Parsers disagree about a backslash in an address (a host that ends at it, or a path that starts there).
    if value.contains('\\') {
        return Err(format!("{label}: The address cannot contain a backslash."));
    }
    let lower = value.to_ascii_lowercase();
    let socks = cfg!(feature = "proxy-transports")
        && ["socks4://", "socks4a://", "socks5://", "socks5h://"]
            .iter()
            .any(|s| lower.starts_with(s));
    if !(lower.starts_with("http://") || lower.starts_with("https://") || socks) {
        #[cfg(feature = "proxy-transports")]
        if value.contains("://") {
            return Err(format!(
                "{label}: Use an HTTP, HTTPS, SOCKS4 or SOCKS5 proxy address."
            ));
        }
        return Err(if value.contains("://") {
            format!(
                "{label}: Only addresses that start with http:// or https:// work here. SOCKS and other kinds are not supported."
            )
        } else {
            format!(
                "{label}: Start the address with http:// or https://, for example http://proxy.example.com:8080."
            )
        });
    }
    let invalid = || {
        format!(
            "{label}: This is not a valid address. It should look like http://proxy.example.com:8080."
        )
    };
    let url = reqwest::Url::parse(value).map_err(|_| invalid())?;
    if url.host_str().is_none_or(str::is_empty) {
        return Err(invalid());
    }
    if !matches!(url.path(), "" | "/") || url.query().is_some() || url.fragment().is_some() {
        return Err(format!(
            "{label}: A proxy address ends after the port. Remove everything after it (for example http://proxy.example.com:8080)."
        ));
    }
    let scheme_len = lower.find("://").map_or(0, |at| at + 3);
    Ok(format!("{}{}", &lower[..scheme_len], &value[scheme_len..]))
}

/// Checks the no-proxy list (comma-separated hosts, domains, addresses and ranges) and returns it as
/// entries joined with commas.
///
/// # Errors
/// A plain-words problem.
pub fn check_no_proxy(label: &str, raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if let Some(problem) = plain_text_problem(value) {
        return Err(format!("{label}: {problem}"));
    }
    let mut entries = Vec::new();
    for entry in value.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        if entry
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '\'' | '"' | '\\'))
        {
            return Err(format!(
                "{label}: Separate the entries with commas. An entry has no spaces, quotes or backslashes."
            ));
        }
        entries.push(entry);
    }
    Ok(entries.join(","))
}

/// The no-proxy list with the addresses of this computer added (those already there are not repeated).
#[must_use]
pub fn with_loopback(list: &str) -> String {
    let mut entries: Vec<&str> = list
        .split(',')
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .collect();
    for local in LOOPBACK {
        if !entries.iter().any(|e| e.eq_ignore_ascii_case(local)) {
            entries.push(local);
        }
    }
    entries.join(",")
}

/// The no-proxy list without the addresses of this computer (what the user typed).
#[must_use]
pub fn without_loopback(list: &str) -> String {
    list.split(',')
        .map(str::trim)
        .filter(|e| !e.is_empty() && !LOOPBACK.iter().any(|l| e.eq_ignore_ascii_case(l)))
        .collect::<Vec<_>>()
        .join(",")
}

fn path_problem(path: &str) -> Option<String> {
    if let Some(problem) = plain_text_problem(path) {
        return Some(problem.to_owned());
    }
    if path.contains("\\\\") {
        return Some(
            "Vibe cannot keep a path with two backslashes in a row (a network path like \\\\server\\share). Copy the file to a folder on this computer and choose it there."
                .to_owned(),
        );
    }
    if !Path::new(path).is_absolute() {
        return Some("Choose the file with the picker, or type its full path.".to_owned());
    }
    None
}

/// Checks the text of a certificate file or folder path without touching the disk and returns it trimmed.
///
/// # Errors
/// A plain-words problem (`label` names the setting).
pub fn check_path_text(label: &str, raw: &str) -> Result<String, String> {
    let path = raw.trim();
    match path_problem(path) {
        Some(problem) => Err(format!("{label}: {problem}")),
        None => Ok(path.to_owned()),
    }
}

/// Checks a certificate file: it exists, can be read, is not a private key and holds at least one PEM
/// certificate that parses.
///
/// # Errors
/// A plain-words problem (`label` names the setting).
pub fn check_certificate_file(label: &str, raw: &str) -> Result<CertCheck, String> {
    let path = raw.trim();
    if let Some(problem) = path_problem(path) {
        return Err(format!("{label}: {problem}"));
    }
    let bytes = read_capped(Path::new(path)).map_err(|problem| format!("{label}: {problem}"))?;
    parse_certificates(&bytes)
        .map(|certificates| CertCheck {
            certificates: certificates.len(),
        })
        .map_err(|problem| format!("{label}: {problem}"))
}

/// The certificates of PEM `bytes`.
///
/// # Errors
/// A plain-words problem.
pub fn parse_certificates(bytes: &[u8]) -> Result<Vec<reqwest::Certificate>, String> {
    if contains(bytes, b"PRIVATE KEY-----") {
        return Err("This file contains a private key. Choose the certificate file (it starts with -----BEGIN CERTIFICATE-----), not a key.".to_owned());
    }
    let certificates = reqwest::Certificate::from_pem_bundle(bytes)
        .map_err(|_| "A certificate in this file is damaged and cannot be read.".to_owned())?;
    if certificates.is_empty() {
        return Err("This file does not hold a PEM certificate. It should contain text that starts with -----BEGIN CERTIFICATE-----. A binary .cer or .der file must first be converted to PEM.".to_owned());
    }
    Ok(certificates)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Reads a file of at most [`MAX_CERT_BYTES`], with a plain problem for each way that fails.
pub(crate) fn read_capped(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read as _;
    let metadata = std::fs::metadata(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "No file exists at that path.".to_owned(),
        std::io::ErrorKind::PermissionDenied => {
            "This file cannot be read (no permission).".to_owned()
        }
        _ => "This file cannot be read.".to_owned(),
    })?;
    if metadata.is_dir() {
        return Err("That path is a folder, not a file. Choose the certificate file.".to_owned());
    }
    if metadata.len() > MAX_CERT_BYTES {
        return Err(
            "This file is larger than 8 MB; a certificate file is a few kilobytes.".to_owned(),
        );
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(MAX_CERT_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::PermissionDenied => {
                "This file cannot be read (no permission).".to_owned()
            }
            _ => "This file cannot be read.".to_owned(),
        })?;
    Ok(bytes)
}

/// Checks a certificate folder: an existing absolute folder path.
///
/// # Errors
/// A plain-words problem.
pub fn check_certificate_dir(label: &str, raw: &str) -> Result<(), String> {
    let path = raw.trim();
    if let Some(problem) = path_problem(path) {
        return Err(format!("{label}: {problem}"));
    }
    match std::fs::metadata(path) {
        Ok(m) if m.is_dir() => Ok(()),
        Ok(_) => Err(format!("{label}: That path is a file, not a folder.")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(format!("{label}: No folder exists at that path."))
        }
        Err(_) => Err(format!("{label}: This folder cannot be read.")),
    }
}
