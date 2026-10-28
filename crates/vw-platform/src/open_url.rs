//! "Open this link in the browser": a link in model output is never followed
//! by the main window; after the user confirmed the full URL in the app's dialog, the UI asks the
//! host to hand it to the system browser, and this module checks it again before any program starts.
//!
//! Depends on: `url` (WHATWG parsing), std (`xdg-open`, `open`, `rundll32 url.dll` with a fixed
//! program name and the canonical URL as one argument; no shell). Called by: the app's `open_link`
//! bridge command.
//! Invariants:
//! - Only `http` and `https` URLs with a host are opened; `javascript:`, `file:`, `data:`, custom
//!   schemes, relative text, whitespace, control characters and over-long input are refused before
//!   anything is spawned.
//! - A URL with a user name or password is refused: `https://bank.example@evil.example/` reads like
//!   the bank but goes to `evil.example` (the UI refuses it too, `ui/src/features/transcript/model/links.ts`).
//! - The URL handed to the OS is the parser's canonical serialisation (percent-encoded, punycode host),
//!   so the opener never sees quotes, spaces or other characters a command line could split on.

use std::ffi::OsString;
use std::process::Command;

use url::Url;

/// Longest accepted URL (the UI uses the same limit).
pub const MAX_URL_LENGTH: usize = 4096;

/// Why a link was not opened.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OpenUrlError {
    /// Empty, too long, or holding whitespace or control characters.
    #[error("the link is empty, too long or contains spaces or control characters")]
    Malformed,
    /// Not a complete absolute URL.
    #[error("the link is not a complete web address")]
    NotAUrl,
    /// A scheme other than http or https.
    #[error("only web links (http, https) can be opened, not `{0}:`")]
    NotWeb(String),
    /// A user name or password before the host.
    #[error("the link hides a user name before the real address; not opened")]
    HasCredentials,
    /// No host.
    #[error("the link has no host")]
    NoHost,
    /// Not a plain `mailto:` link to one address (such as the About contact line).
    #[error("only a mailto: link to one plain address can be opened here")]
    NotMail,
    /// The browser could not be started.
    #[error("could not start the browser: {0}")]
    Spawn(String),
}

/// Checks `text` and returns the canonical URL that may be opened.
///
/// # Errors
/// Every [`OpenUrlError`] except `Spawn`, for the reasons named in the module docs.
pub fn web_url(text: &str) -> Result<String, OpenUrlError> {
    if text.is_empty()
        || text.len() > MAX_URL_LENGTH
        || text.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(OpenUrlError::Malformed);
    }
    let url = Url::parse(text).map_err(|_| OpenUrlError::NotAUrl)?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(OpenUrlError::NotWeb(url.scheme().to_owned()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(OpenUrlError::HasCredentials);
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(OpenUrlError::NoHost);
    }
    Ok(url.into())
}

/// The program that opens a URL in the default browser on this platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkOpener {
    /// Windows: `rundll32.exe url.dll,FileProtocolHandler <url>` (the shell's URL handler, no `cmd`).
    UrlDll,
    /// macOS: `open <url>`.
    Open,
    /// Linux and other Unix desktops: `xdg-open <url>`.
    XdgOpen,
}

impl LinkOpener {
    /// The opener for the platform this build targets.
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(windows) {
            Self::UrlDll
        } else if cfg!(target_os = "macos") {
            Self::Open
        } else {
            Self::XdgOpen
        }
    }

    /// Program and arguments that open `url` (already checked by [`web_url`]).
    #[must_use]
    pub fn command(self, url: &str) -> (&'static str, Vec<OsString>) {
        match self {
            Self::UrlDll => (
                "rundll32.exe",
                vec![
                    OsString::from("url.dll,FileProtocolHandler"),
                    OsString::from(url),
                ],
            ),
            Self::Open => ("open", vec![OsString::from(url)]),
            Self::XdgOpen => ("xdg-open", vec![OsString::from(url)]),
        }
    }
}

/// Longest accepted e-mail address (RFC 5321's path limit).
pub const MAX_MAIL_LENGTH: usize = 254;

/// Checks a `mailto:` link (for example, Settings › About contact): exactly `mailto:` plus one plain address
/// (`local@domain.tld`, letters, digits and `._%+-` before the `@`, a dotted host after it), with no query (no
/// subject, body, cc), no fragment and no second address. Returns the link unchanged.
///
/// # Errors
/// [`OpenUrlError::NotMail`] for anything else (nothing is started then).
pub fn mail_url(text: &str) -> Result<String, OpenUrlError> {
    let address = text.strip_prefix("mailto:").ok_or(OpenUrlError::NotMail)?;
    let (local, domain) = address.split_once('@').ok_or(OpenUrlError::NotMail)?;
    let local_ok = !local.is_empty()
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-'));
    let labels: Vec<&str> = domain.split('.').collect();
    let domain_ok = labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    if address.len() > MAX_MAIL_LENGTH || !local_ok || !domain_ok {
        return Err(OpenUrlError::NotMail);
    }
    Ok(text.to_owned())
}

/// Checks `text` with [`mail_url`] and hands it to the system's mail program (the same opener as web links).
///
/// # Errors
/// [`OpenUrlError::NotMail`] (nothing is started then); [`OpenUrlError::Spawn`] when the opener cannot start.
pub fn open_mail_url(text: &str) -> Result<(), OpenUrlError> {
    let url = mail_url(text)?;
    spawn_opener(&url)
}

/// Starts the platform opener for an already checked link and reaps it in the background.
fn spawn_opener(url: &str) -> Result<(), OpenUrlError> {
    let (program, args) = LinkOpener::current().command(url);
    let mut command = Command::new(program);
    crate::child_env::remove_own_run_settings(&mut command);
    let mut child = command
        .args(args)
        .spawn()
        .map_err(|error| OpenUrlError::Spawn(error.to_string()))?;
    // Reap the opener when it exits (it hands the link to the browser or mail program and ends at once).
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Checks `text` with [`web_url`] and opens it in the system browser.
///
/// # Errors
/// The [`web_url`] refusals (nothing is started then); [`OpenUrlError::Spawn`] when the opener
/// program cannot start.
pub fn open_web_url(text: &str) -> Result<(), OpenUrlError> {
    let url = web_url(text)?;
    spawn_opener(&url)
}
