//! Which URLs a session's browser may show:
//! public `http`/`https` hosts, and on the loopback interface only plain `http` on ports of its own session; never
//! private or special addresses, other schemes, or the app's own origins.
//!
//! Depends on: url, vw-netguard (address classes, canary suffix). Called by: `service` (open, navigation checks),
//! `hooks`, vw-core-api (the agent tools' refusals, the address bar).
//! Invariants:
//! - Loopback means exactly `127.0.0.1`, `localhost` or `[::1]` after URL normalisation; other spellings
//!   (`localhost.`, `[::ffff:127.0.0.1]`, `127.0.0.2`) are refused. Loopback is `http` only and needs a port of the
//!   session (the caller's owner check) and is never the app's development origin.
//! - An IP literal must be public (vw-netguard classes), or a private-network address (RFC 1918 / ULA / carrier-grade
//!   NAT) the USER typed (Lan: never the agent, never an in-page navigation; the Debug API acts as
//!   the user by design). Link-local (`169.254.0.0/16`, `fe80::/10`, cloud metadata) is refused for
//!   every origin; a host NAME is checked against DNS on every navigation
//!   (the hooks' cached resolver; the core before an open) and again at request time by the network guard where it
//!   runs; `*.localhost` is the app's own and is refused.
//! - Public pages open on every platform (the baseline, ShellX's level: private, LAN, file and app-own targets refused
//!   here, names checked against DNS by the core before an open). Where the network guard runs (Linux) it checks
//!   every request again at request time; it is an extra layer, never a condition for loading. A canary URL
//!   (`*.vw-guard.invalid`, RFC 6761 `.invalid`) is allowed only as the one the engine announced for the mount's
//!   pending guard check (the hooks decide that); any other is refused.
//! - `file:`, `data:`, `blob:`, `javascript:`, `about:` other than `about:blank`, every custom scheme and any URL with
//!   user info are refused.

use url::{Host, Url};
use vw_netguard::CANARY_SUFFIX;
use vw_netguard::ip::{IpClass, classify};

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
    /// leased to the session: the owner is only inferred (confidence `inferred`). Refused for the agent;
    /// the user may still open it from the app's own window.
    NotProven,
}

/// Where a URL goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    /// `about:blank`.
    Blank,
    /// This computer, on `port` (owner rules apply).
    Loopback {
        /// The port.
        port: u16,
    },
    /// A public host (checked again at request time by the network guard).
    Public,
    /// A private-network literal (RFC 1918 / ULA, carrier-grade NAT; never link-local): opened only when the user
    /// typed it (the address bar; the Debug API acts as the user by design), never by the agent or an
    /// in-page navigation.
    Lan,
    /// The network guard's canary (`http://<token>.vw-guard.invalid/`).
    Canary,
}

/// What kind of page a webview mount holds for its whole life (Private Network Access model): a
/// page from the web never shares a mount with a page of the session's own local server, so the network guard can
/// judge every request by the mount it came from (a mount's listener) instead of by a page that may be on its way
/// out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageClass {
    /// Public pages (and `about:blank`): nothing on this computer is reachable.
    Web,
    /// Pages of this session's own servers: their ports and other sessions' servers (background) are reachable.
    Local,
    /// A page on the user's local network the USER opened (a router's admin page): its own host is reachable; the
    /// agent neither reads nor acts on it.
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
    #[error("only http:// and https:// web addresses open in the browser (got {0}:)")]
    Scheme(String),
    /// `https` on this computer.
    #[error("servers on this computer open with http:// only (got https)")]
    LoopbackHttps,
    /// User info in the URL.
    #[error("URLs with a user name or password are not opened")]
    UserInfo,
    /// A host that is not allowed (the text names the class).
    #[error("{0} is not opened: {1}")]
    Host(String, &'static str),
    /// A private-network address for the agent or an in-page navigation (only the user opens those).
    #[error(
        "{0} is on your local network: the agent does not open local-network pages (you can, from the address bar)"
    )]
    Lan(String),
    /// A name reserved for this computer or the app.
    #[error("{0} is reserved for this computer and is not opened")]
    ReservedName(String),
    /// A navigation inside a mount to the other page class (from a web page to a local server or back, top level
    /// or in a frame): only a fresh load from the address bar or the agent's navigate opens it.
    #[error(
        "pages from the web and this session's local servers do not share a page: open {0} from the address bar \
         (or with browser_act navigate)"
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
         session; start the server with process_start (or on a leased port) to open it"
    )]
    NotProven(u16),
    /// Nothing of this session is on the port.
    #[error(
        "port {0} is not one of this session's servers or leased ports (processes_list shows them)"
    )]
    Unattributed(u16),
}

/// The fixed part of the policy (no session data).
#[derive(Clone, Debug, Default)]
pub struct UrlPolicy {
    /// The app's development origin (`devUrl`), refused by port when its host is loopback. `None` in user builds.
    pub dev_origin: Option<Url>,
}

impl UrlPolicy {
    /// Parses `text` and classifies it (everything except port ownership and the guard).
    ///
    /// # Errors
    /// The reason, as a [`UrlRefusal`].
    pub fn shape(&self, text: &str) -> Result<(Url, Destination), UrlRefusal> {
        let url = Url::parse(text.trim()).map_err(|_| UrlRefusal::NotAUrl(short(text)))?;
        self.classify(&url).map(|d| (url, d))
    }

    /// Classifies a parsed URL (scheme, user info, host, app origin).
    ///
    /// # Errors
    /// The reason, as a [`UrlRefusal`].
    pub fn classify(&self, url: &Url) -> Result<Destination, UrlRefusal> {
        if url.as_str() == "about:blank" {
            return Ok(Destination::Blank);
        }
        let scheme = url.scheme();
        if scheme != "http" && scheme != "https" {
            return Err(UrlRefusal::Scheme(short(scheme)));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(UrlRefusal::UserInfo);
        }
        let port = url.port_or_known_default().unwrap_or(80);
        let loopback = match url.host() {
            Some(Host::Ipv4(v4)) => {
                if v4.octets() == [127, 0, 0, 1] {
                    true
                } else {
                    return literal(&v4.to_string(), classify(v4.into()));
                }
            }
            Some(Host::Ipv6(v6)) => {
                if v6.is_loopback() {
                    true
                } else {
                    return literal(&format!("[{v6}]"), classify(v6.into()));
                }
            }
            Some(Host::Domain(name)) => {
                let lower = name.to_ascii_lowercase();
                if lower == "localhost" {
                    true
                } else if lower.ends_with(CANARY_SUFFIX) && scheme == "http" {
                    return Ok(Destination::Canary);
                } else if lower == "localhost."
                    || lower.trim_end_matches('.').ends_with(".localhost")
                {
                    return Err(UrlRefusal::ReservedName(short(&lower)));
                } else {
                    return Ok(Destination::Public);
                }
            }
            None => return Err(UrlRefusal::NotAUrl(short(url.as_str()))),
        };
        debug_assert!(loopback);
        if scheme != "http" {
            return Err(UrlRefusal::LoopbackHttps);
        }
        if self.is_app_origin_port(port) {
            return Err(UrlRefusal::AppOrigin(port));
        }
        Ok(Destination::Loopback { port })
    }

    fn is_app_origin_port(&self, port: u16) -> bool {
        // Every loopback spelling reaches the same server, so the port alone decides once the dev origin is
        // itself on loopback (it always is: `devUrl` = http://127.0.0.1:1420).
        self.dev_origin.as_ref().is_some_and(|dev| {
            let dev_on_loopback = match dev.host() {
                Some(Host::Ipv4(v)) => v.is_loopback(),
                Some(Host::Ipv6(v)) => v.is_loopback(),
                Some(Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
                None => false,
            };
            dev_on_loopback && dev.port_or_known_default() == Some(port)
        })
    }

    /// The full check for a session: shape, then the owner of a loopback port. Public hosts pass here (the core
    /// checks names against DNS, the guard every request where it runs); a private-network literal passes only when
    /// `user` (typed by the user: the address bar or the Debug API).
    ///
    /// # Errors
    /// The reason, as a [`UrlRefusal`].
    pub fn check(
        &self,
        text: &str,
        owner_of: impl FnOnce(u16) -> PortOwner,
        user: bool,
    ) -> Result<(Url, Destination), UrlRefusal> {
        let (url, destination) = self.shape(text)?;
        match destination {
            Destination::Lan if user => Ok((url, destination)),
            Destination::Lan => Err(UrlRefusal::Lan(short(url.host_str().unwrap_or("")))),
            Destination::Loopback { port } => match owner_of(port) {
                PortOwner::ThisSession => Ok((url, destination)),
                PortOwner::OtherSession(owner) | PortOwner::OtherApp(owner) => {
                    Err(UrlRefusal::NotThisSession { port, owner })
                }
                PortOwner::Nobody => Err(UrlRefusal::Unattributed(port)),
                PortOwner::NotProven => Err(UrlRefusal::NotProven(port)),
            },
            Destination::Public => Ok((url, destination)),
            Destination::Blank | Destination::Canary => Err(UrlRefusal::NotAUrl(short(text))),
        }
    }

    /// The navigation decision inside a webview mounted with page `class`: `about:blank`, loopback ports in
    /// `allowed` (Local mounts), public hosts (Web mounts), and in a Lan mount only its own host `lan_host` (the one
    /// the user opened); never the other page class (a fresh mount opens that), never an unannounced canary (the
    /// hooks allow the announced one before asking here). Returns the refusal.
    ///
    /// # Errors
    /// The reason, as a [`UrlRefusal`].
    pub fn navigation(
        &self,
        url: &Url,
        allowed: &[u16],
        class: PageClass,
        lan_host: Option<&str>,
    ) -> Result<(), UrlRefusal> {
        let destination = self.classify(url)?;
        let same_lan_host =
            class == PageClass::Lan && lan_host.is_some_and(|h| url.host_str() == Some(h));
        match destination {
            Destination::Blank => return Ok(()),
            Destination::Canary => {
                return Err(UrlRefusal::ReservedName(short(
                    url.host_str().unwrap_or(""),
                )));
            }
            Destination::Loopback { port } if !allowed.contains(&port) => {
                return Err(UrlRefusal::Unattributed(port));
            }
            Destination::Lan | Destination::Public if same_lan_host => return Ok(()),
            Destination::Lan => return Err(UrlRefusal::Lan(short(url.host_str().unwrap_or("")))),
            _ => {}
        }
        if destination.class() == class {
            Ok(())
        } else {
            Err(UrlRefusal::OtherClass(short(url.as_str())))
        }
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

/// The refusal of a non-loopback IP literal that is not public.
fn literal(text: &str, class: IpClass) -> Result<Destination, UrlRefusal> {
    match class {
        IpClass::Public => Ok(Destination::Public),
        // Link-local (169.254.0.0/16, fe80::/10) stays refused for everyone, user-typed too: it holds cloud metadata
        // endpoints and no user needs to type one.
        IpClass::Private | IpClass::SharedCgnat => Ok(Destination::Lan),
        _ => Err(UrlRefusal::Host(short(text), class.describe())),
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
