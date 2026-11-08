//! Browser URL admission: web schemes and credentials are checked independently of address space.
//! Local and private destinations use ordinary browser permissions. Backend capture uses `fetch_policy`.

use url::{Host, Url};
use vw_netguard::CANARY_SUFFIX;
use vw_netguard::ip::classify;

/// Who a port belongs to, as seen by the asking session (computed by the core from attribution and leases).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortOwner {
    /// Attributed to or leased by the asking session.
    ThisSession,
    /// Attributed to or leased by another session (its display name: "another session ('Shop')").
    OtherSession(String),
    /// Another program listens there ("another app (node)").
    OtherApp(String),
    /// Nothing listens there and no session holds it.
    Nobody,
    /// Something in this session's folder listens there, but Vibe Work did not start it and the port is not
    /// leased to the session: the owner is only inferred (confidence `inferred`). Pinned App previews require
    /// proven ownership; browser navigation does not consult port ownership.
    NotProven,
}

/// Where a URL goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    /// `about:blank`.
    Blank,
    /// This computer, on `port`.
    Loopback {
        /// The port.
        port: u16,
    },
    /// A host name or public address.
    Public,
    /// A non-public literal address; ordinary browser admission applies.
    Lan,
    /// The network guard's canary (`http://<token>.vw-guard.invalid/`).
    Canary,
}

/// Original mount address class. Used for storage and the separate backend capture policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageClass {
    /// A host name or public address.
    Web,
    /// A loopback address.
    Local,
    /// A non-public literal address.
    Lan,
}

impl Destination {
    /// The page class a mount for this destination gets.
    #[must_use]
    pub fn class(self) -> PageClass {
        match self {
            Self::Loopback { .. } => PageClass::Local,
            Self::Lan => PageClass::Lan,
            Self::Blank | Self::Public | Self::Canary => PageClass::Web,
        }
    }
}

/// Why a URL is refused (the text reaches the agent and the UI).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UrlRefusal {
    /// Not a URL at all.
    #[error("'{0}' is not a URL")]
    NotAUrl(String),
    /// A scheme other than `http`/`https`.
    #[error("This address uses {0}:. Open an http:// or https:// web address instead")]
    Scheme(String),
    /// `https` on this computer.
    #[error("This server opens with http://. Change the address and try again")]
    LoopbackHttps,
    /// User info in the URL.
    #[error("URLs with a user name or password are not opened")]
    UserInfo,
    /// A host that is not allowed (the text names the class).
    #[error("{0} is not opened: {1}")]
    Host(String, &'static str),
    /// A backend fetch outside the current page's existing local-media scope.
    #[error("the backend media fetch cannot reach local-network address {0} from this page")]
    Lan(String),
    /// A name reserved for this computer or the app.
    #[error("{0} is reserved for this computer and is not opened")]
    ReservedName(String),
    /// A backend fetch or pinned App preview operation outside its existing resource scope.
    #[error(
        "this operation is limited to the current page's resource scope: open {0} in a browser tab instead"
    )]
    OtherClass(String),
    /// The app's own development origin.
    #[error("port {0} is Vibe Work's own development server and cannot be opened")]
    AppOrigin(u16),
    /// The port belongs to someone else.
    #[error("port {port} belongs to {owner}; a session opens only its own ports on this computer")]
    NotThisSession {
        /// The port.
        port: u16,
        /// Owner text.
        owner: String,
    },
    /// Only inferred to be this session's.
    #[error(
        "port {0} runs in this session's folder but was not started by Vibe Work and is not leased to this \
         session; start the server in this session's Terminal to open it"
    )]
    NotProven(u16),
    /// Nothing of this session is on the port.
    #[error(
        "port {0} is not one of this session's servers. Start the server in this session's Terminal, then open its address"
    )]
    Unattributed(u16),
}

/// The fixed part of the policy (no session data).
#[derive(Clone, Debug, Default)]
pub struct UrlPolicy {
    /// The app's development origin (`devUrl`), reserved by backend fetch policy. Browser admission ignores it.
    pub dev_origin: Option<Url>,
}

impl UrlPolicy {
    /// Parses `text` and classifies its address for storage and backend resource scope.
    ///
    /// # Errors
    /// The reason, as a [`UrlRefusal`].
    pub fn shape(&self, text: &str) -> Result<(Url, Destination), UrlRefusal> {
        let url = Url::parse(text.trim()).map_err(|_| UrlRefusal::NotAUrl(short(text)))?;
        self.classify(&url).map(|d| (url, d))
    }

    /// Checks scheme and user info, then classifies the host without restricting its address space.
    ///
    /// # Errors
    /// The reason, as a [`UrlRefusal`].
    pub fn classify(&self, url: &Url) -> Result<Destination, UrlRefusal> {
        if url.as_str() == "about:blank" {
            return Ok(Destination::Blank);
        }
        if !matches!(url.scheme(), "http" | "https") {
            return Err(UrlRefusal::Scheme(short(url.scheme())));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(UrlRefusal::UserInfo);
        }
        if url.path().starts_with("/__vw_guard/")
            && matches!(url.host(), Some(Host::Ipv4(ip)) if ip.is_loopback())
        {
            return Err(UrlRefusal::ReservedName("browser network canary".into()));
        }
        let port = url.port_or_known_default().unwrap_or(80);
        match url.host() {
            Some(Host::Domain(name)) if name.ends_with(CANARY_SUFFIX) && url.scheme() == "http" => {
                Ok(Destination::Canary)
            }
            Some(Host::Domain(name))
                if name.trim_end_matches('.').eq_ignore_ascii_case("localhost")
                    || name.trim_end_matches('.').ends_with(".localhost") =>
            {
                Ok(Destination::Loopback { port })
            }
            Some(Host::Ipv4(ip)) if ip.is_loopback() => Ok(Destination::Loopback { port }),
            Some(Host::Ipv6(ip)) if ip.is_loopback() => Ok(Destination::Loopback { port }),
            Some(Host::Ipv4(ip)) if !classify(ip.into()).is_public() => Ok(Destination::Lan),
            Some(Host::Ipv6(ip)) if !classify(ip.into()).is_public() => Ok(Destination::Lan),
            Some(_) => Ok(Destination::Public),
            None => Err(UrlRefusal::NotAUrl(short(url.as_str()))),
        }
    }

    /// Browser address admission. Site permissions, tab ownership and access modes are checked by the caller.
    /// Legacy port/grant arguments do not restrict browser destinations.
    ///
    /// # Errors
    /// Malformed URLs, credentials, non-web schemes, or an internal guard canary.
    pub fn check(
        &self,
        text: &str,
        _owner_of: impl FnOnce(u16) -> PortOwner,
        _user: bool,
    ) -> Result<(Url, Destination), UrlRefusal> {
        let (url, destination) = self.shape(text)?;
        match destination {
            Destination::Blank | Destination::Canary => Err(UrlRefusal::NotAUrl(short(text))),
            _ => Ok((url, destination)),
        }
    }

    /// Browser navigation keeps URL and document checks without an address-space restriction.
    ///
    /// # Errors
    /// An unsupported URL or an unannounced internal guard canary.
    pub fn navigation(
        &self,
        url: &Url,
        _allowed: &[u16],
        _class: PageClass,
        _lan_host: Option<&str>,
    ) -> Result<(), UrlRefusal> {
        if self.classify(url)? == Destination::Canary {
            return Err(UrlRefusal::ReservedName(short(
                url.host_str().unwrap_or(""),
            )));
        }
        Ok(())
    }

    /// The navigation decision for a frame of a page (never the page itself): [`Self::navigation`], plus the documents
    /// a page makes itself: the inline document `about:srcdoc` (exactly that address), `data:` and `blob:`. Returns the
    /// refusal.
    ///
    /// # Errors
    /// The reason, as a [`UrlRefusal`].
    pub fn frame_navigation(
        &self,
        url: &Url,
        allowed: &[u16],
        class: PageClass,
        lan_host: Option<&str>,
    ) -> Result<(), UrlRefusal> {
        if url.as_str() == "about:srcdoc" || matches!(url.scheme(), "data" | "blob") {
            return Ok(());
        }
        self.navigation(url, allowed, class, lan_host)
    }

    /// [`Self::navigation`] as a yes/no.
    #[must_use]
    pub fn allows_navigation(
        &self,
        url: &Url,
        allowed: &[u16],
        class: PageClass,
        lan_host: Option<&str>,
    ) -> bool {
        self.navigation(url, allowed, class, lan_host).is_ok()
    }

    /// True for the network guard's canary URL (never shown, never recorded as the page).
    #[must_use]
    pub fn is_canary(&self, url: &Url) -> bool {
        self.classify(url) == Ok(Destination::Canary)
    }
}

/// `http://127.0.0.1:<port><path>` for a port and an optional path (a leading `/` is added).
///
/// # Errors
/// [`UrlRefusal::NotAUrl`] when `path` makes the URL invalid.
pub fn loopback_url(port: u16, path: Option<&str>) -> Result<String, UrlRefusal> {
    let path = path.unwrap_or("/").trim();
    let path = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    let text = format!("http://127.0.0.1:{port}{path}");
    let url = Url::parse(&text).map_err(|_| UrlRefusal::NotAUrl(short(&text)))?;
    if url.port_or_known_default() != Some(port) || url.host_str() != Some("127.0.0.1") {
        return Err(UrlRefusal::NotAUrl(short(&text)));
    }
    Ok(url.into())
}

/// At most 80 characters of untrusted text for a refusal.
fn short(text: &str) -> String {
    text.chars().take(80).collect()
}

/// These documents belong to the page itself and need no refusal receipt or notification.
pub(crate) fn embedded_document(url: &Url) -> bool {
    url.scheme() == "data" || (url.scheme() == "about" && matches!(url.path(), "srcdoc" | "blank"))
}

#[cfg(test)]
mod local_address_tests {
    use super::*;

    #[test]
    fn browser_addresses_do_not_need_a_local_network_grant() {
        let private = format!("http://{}/", std::net::Ipv4Addr::new(192, 168, 1, 2));
        for url in [
            private.as_str(),
            "http://localhost:9999/",
            "https://127.0.0.2/",
            "http://169.254.1.1/",
            "http://[fe80::1]/",
            "http://nas.local/",
        ] {
            assert!(
                UrlPolicy::default()
                    .check(url, |_| PortOwner::Nobody, false)
                    .is_ok(),
                "{url}"
            );
        }
    }
}
