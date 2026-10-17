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
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
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
    let scheme_len = if lower.starts_with("https://") { 8 } else { 7 };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_shown_from_its_parsed_parts_only() {
        assert_eq!(
            mask_proxy_url("http://user:secret@proxy.corp:8080"),
            "http://proxy.corp:8080"
        );
        assert_eq!(mask_proxy_url("https://user@p:1"), "https://p:1");
        assert_eq!(mask_proxy_url("http://u:p@a@host:1"), "http://host:1");
        assert_eq!(mask_proxy_url("http://proxy.corp"), "http://proxy.corp:80");
        assert_eq!(
            mask_proxy_url("HTTPS://Proxy.Corp:443/x?y#z"),
            "https://proxy.corp:443"
        );
        assert_eq!(mask_proxy_url("http://[::1]:3128"), "http://[::1]:3128");
        // The environment often has no scheme; it reads as http.
        assert_eq!(
            mask_proxy_url("user:pw@proxy.corp:3128"),
            "http://proxy.corp:3128"
        );
        assert_eq!(mask_proxy_url("proxy.corp:3128"), "http://proxy.corp:3128");
        // What does not parse is not shown, and never partly.
        for odd in ["", "http://", "user:pa/ss#w?d@host:1", "http://u:pw@:80"] {
            let shown = mask_proxy_url(odd);
            assert_eq!(shown, "(address not shown)", "{odd}");
        }
        assert_eq!(
            form_proxy_url("http://user:secret@proxy.corp:8080"),
            "http://***@proxy.corp:8080"
        );
        assert_eq!(
            form_proxy_url("http://proxy.corp:8080"),
            "http://proxy.corp:8080"
        );
        assert!(
            has_sign_in("http://u:p@h") && has_sign_in("http://u@h") && !has_sign_in("http://h:1")
        );
        assert!(has_sign_in("not an address @ all"));
    }

    #[test]
    fn a_hand_made_address_cannot_show_another_host_than_the_one_connected_to() {
        // The text after `#`, `?` or `/` is not the host: reqwest connects to evil.example, so that is what shows,
        // and the address is refused anyway (an address ends after the port).
        for trick in [
            "http://evil.example:3128#@proxy.corp:8080",
            "http://evil.example:3128?@proxy.corp:8080",
            "http://evil.example:3128/@proxy.corp:8080",
            "http://evil.example:3128\\@proxy.corp:8080",
        ] {
            let shown = mask_proxy_url(trick);
            assert!(!shown.contains("proxy.corp"), "{trick} -> {shown}");
            assert_eq!(shown, "http://evil.example:3128", "{trick}");
            assert!(check_proxy_url("HTTPS proxy", trick).is_err(), "{trick}");
            let url = reqwest::Url::parse(trick).expect("parses");
            assert_eq!(url.host_str(), Some("evil.example"), "{trick}");
        }
    }

    #[test]
    fn scrub_removes_a_sent_value_from_foreign_text() {
        let raw = "http://user:secret@proxy:8080";
        let out = scrub("cannot use 'http://user:secret@proxy:8080' here", &[raw]);
        assert!(!out.contains("secret") && !out.contains("user"), "{out}");
        assert!(out.contains("http://proxy:8080"), "{out}");
    }

    #[test]
    fn proxy_addresses_are_checked_in_plain_words() {
        assert_eq!(
            check_proxy_url("HTTPS proxy", "  HTTP://u:p@Proxy.corp:8080 ").as_deref(),
            Ok("http://u:p@Proxy.corp:8080")
        );
        for (input, fragment) in [
            ("proxy.corp:8080", "Start the address with http://"),
            ("socks5://proxy:1080", "SOCKS"),
            ("http://", "not a valid address"),
            ("http://proxy:99999", "not a valid address"),
            ("http://proxy :8080", "spaces"),
            ("http://proxy:8080/path", "ends after the port"),
            ("http://u:${HOME}@proxy:8080", "${"),
            ("http://proxy\n.corp:8080", "control"),
        ] {
            let problem = check_proxy_url("HTTPS proxy", input).expect_err(input);
            assert!(problem.contains(fragment), "{input}: {problem}");
            assert!(
                !problem.contains("proxy.corp"),
                "never quotes the value: {problem}"
            );
        }
    }

    #[test]
    fn a_problem_never_repeats_the_sign_in() {
        let problem =
            check_proxy_url("HTTP proxy", "hunter2:secret@proxy:8080").expect_err("no scheme");
        assert!(
            !problem.contains("hunter2") && !problem.contains("secret"),
            "{problem}"
        );
    }

    #[test]
    fn no_proxy_lists_and_the_addresses_of_this_computer() {
        assert_eq!(
            check_no_proxy("No-proxy list", " .corp.example , 10.0.0.0/8,,intranet "),
            Ok(".corp.example,10.0.0.0/8,intranet".to_owned())
        );
        assert!(check_no_proxy("No-proxy list", "a b").is_err());
        assert_eq!(with_loopback(""), "localhost,127.0.0.1,::1");
        assert_eq!(with_loopback("a,LOCALHOST"), "a,LOCALHOST,127.0.0.1,::1");
        assert_eq!(without_loopback("a,localhost,127.0.0.1,::1,b"), "a,b");
        assert_eq!(without_loopback(&with_loopback("x")), "x");
    }

    #[test]
    fn certificate_paths_are_checked_before_the_file_is_read() {
        let unc = check_certificate_file("Certificate file", "\\\\server\\share\\ca.pem")
            .expect_err("unc");
        assert!(unc.contains("two backslashes"), "{unc}");
        let relative = check_certificate_file("Certificate file", "ca.pem").expect_err("relative");
        assert!(relative.contains("full path"), "{relative}");
        // An absolute path on this computer: Windows needs a drive, so `/definitely/...` is not absolute there.
        let absent = if cfg!(windows) {
            "C:\\definitely\\not\\here.pem"
        } else {
            "/definitely/not/here.pem"
        };
        let missing = check_certificate_file("Certificate file", absent).expect_err("missing");
        assert!(missing.contains("No file exists"), "{missing}");
    }

    #[test]
    fn certificate_content_problems_are_told_apart() {
        let key =
            parse_certificates(b"-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n");
        assert!(key.expect_err("key").contains("private key"));
        let text = parse_certificates(b"hello, this is not a certificate\n");
        assert!(
            text.expect_err("text")
                .contains("does not hold a PEM certificate")
        );
        let damaged =
            parse_certificates(b"-----BEGIN CERTIFICATE-----\n!!!!\n-----END CERTIFICATE-----\n");
        assert!(damaged.expect_err("damaged").contains("damaged"));
    }
}
