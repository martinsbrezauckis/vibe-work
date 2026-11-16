//! Browser subrequest admission. Depends on live mount identity and supported schemes.
//! The engine resolves names under its normal network permissions; address literals of this computer and the local
//! network follow Local Network Access per webview (`local_access`): Vibe Work's own ports are refused for every
//! page (also on this computer's own interface addresses), the App preview reaches the servers of this chat and of
//! other chats on this computer (`State::chat_ports`; never an unattributed port, the local network or link-local),
//! and a page from the web whose site the person blocked is refused here. An undecided site's request goes on to the
//! network guard, which asks the person.
use super::Hooks;
use crate::policy::{Destination, PageClass};
use url::Url;
impl Hooks {
    pub(super) fn request_pre(&self, label: &str, url: &Url) -> Result<(), ()> {
        {
            let state = self.0.lock();
            let session = state.session_of(label).ok_or(())?;
            if state
                .entries
                .get(session)
                .and_then(|e| e.local_document.as_ref())
                .is_some_and(|approved| crate::same_local_document(approved, url))
            {
                return Ok(());
            }
        }
        let mut probe = url.clone();
        match url.scheme() {
            "data" | "blob" => return Ok(()),
            "ws" => {
                let _ = probe.set_scheme("http");
            }
            "wss" => {
                let _ = probe.set_scheme("https");
            }
            _ => {}
        }
        let destination = self.0.policy.classify(&probe).map_err(|_| ())?;
        if self.own_address_internal_port(&probe) {
            return Err(());
        }
        self.local_barrier(label, destination)
    }

    /// True for a request to one of Vibe Work's own ports on one of this computer's own interface addresses (this
    /// computer too, whatever the address's class).
    fn own_address_internal_port(&self, url: &Url) -> bool {
        let ip = match url.host() {
            Some(url::Host::Ipv4(ip)) => std::net::IpAddr::V4(ip),
            Some(url::Host::Ipv6(ip)) => std::net::IpAddr::V6(ip),
            _ => return false,
        };
        let port = url.port_or_known_default().unwrap_or(80);
        vw_netguard::own::is_own(ip)
            && self
                .0
                .internal_ports()
                .is_some_and(|internal| internal.contains(&port))
    }

    /// Local Network Access for one webview's request to an address literal (module docs).
    fn local_barrier(&self, label: &str, destination: Destination) -> Result<(), ()> {
        let internal = self.0.internal_ports();
        let lan = match destination {
            Destination::Loopback { port } => {
                if internal
                    .as_ref()
                    .is_some_and(|internal| internal.contains(&port))
                {
                    return Err(());
                }
                None
            }
            Destination::Lan => Some(()),
            Destination::Blank | Destination::Public | Destination::Canary => return Ok(()),
        };
        let state = self.0.lock();
        let session = state.session_of(label).ok_or(())?;
        let entry = state.entries.get(session).ok_or(())?;
        let pinned_app =
            state.tabs.contains_key(session) && !state.page_owners.contains_key(session);
        if pinned_app {
            return match destination {
                Destination::Loopback { port }
                    if lan.is_none()
                        && app_port_admitted(&state, session, internal.as_deref(), port) =>
                {
                    Ok(())
                }
                _ => Err(()),
            };
        }
        if entry.mount_class() != PageClass::Web {
            return Ok(());
        }
        let site = entry.url.as_ref().and_then(crate::local_access::site_of);
        drop(state);
        let blocked = site.is_some_and(|site| {
            self.0
                .site_answers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&site)
                == Some(crate::local_access::SiteAnswer::Block)
        });
        if blocked { Err(()) } else { Ok(()) }
    }
}

/// The one live admission of a loopback destination port for the App preview of `session`, asked before the engine
/// sends the request, for its subrequests and its document and frame navigations alike: a chat's server the core's
/// latest scan attributed to the port and that still holds it now (`State::chat_port_allowed`, a fresh answer each
/// time), never one of Vibe Work's own (`internal`). A port in the kept open list whose server left is refused here
/// before the periodic re-check withdraws it, so another program's listener there is never reached.
pub(super) fn app_port_admitted(
    state: &crate::state::State,
    session: &str,
    internal: Option<&[u16]>,
    port: u16,
) -> bool {
    state.chat_port_allowed(session, internal, port)
}

/// App preview remains pinned to the session's servers; browser tabs allow every HTTP address class.
pub(super) fn page_allowed(
    policy: &crate::policy::UrlPolicy,
    entry: &crate::state::Entry,
    pinned_app: bool,
    url: &Url,
) -> bool {
    if !pinned_app
        && entry
            .local_document
            .as_ref()
            .is_some_and(|approved| crate::same_local_document(approved, url))
    {
        return true;
    }
    if pinned_app {
        policy
            .fetch_navigation(url, &entry.allowed, crate::policy::PageClass::Local, None)
            .is_ok()
    } else {
        policy.allows_navigation(
            url,
            &entry.allowed,
            entry.mount_class(),
            entry.mount_lan_host(),
        )
    }
}
