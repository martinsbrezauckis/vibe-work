//! Per-mount subrequest policy for the preview hooks. Uses registry identity, URL classes and DNS checks.
//! Called by the request barrier; revoked or unknown mounts have no request authority.
use super::{Hooks, NameCheck, name_to_check};
use crate::policy::{Destination, PageClass};
use url::Url;

impl Hooks {
    /// The policy part of a sub-request decision: `Ok(None)` send it, `Ok(Some(check))` only if the name's verdict
    /// allows it ([`NameCheck`]), `Err(())` refuse it.
    pub(super) fn request_pre(&self, label: &str, url: &Url) -> Result<Option<NameCheck>, ()> {
        // Failed/closed mounts have no request authority, even for public URLs and late deferrals.
        if self.0.lock().session_of(label).is_none() {
            return Err(());
        }
        // WebSockets are judged like their HTTP equivalent; `data:` / `blob:` never leave the page.
        let mut probe = url.clone();
        match url.scheme() {
            "data" | "blob" => return Ok(None),
            "ws" => {
                let _ = probe.set_scheme("http");
            }
            "wss" => {
                let _ = probe.set_scheme("https");
            }
            _ => {}
        }
        let port = match self.0.policy.classify(&probe) {
            Ok(Destination::Canary | Destination::Blank) => return Ok(None),
            // A host NAME goes through the same cached resolver as navigations (a public
            // page's `<img src="http://nas.lan/">` must not reach the home network), failing closed; the Lan page's
            // own host is looked up too and may only point to the local network. The engine resolves again when it connects, so a rebinding name can still
            // change its answer in between: only the Linux guard pins the address.
            Ok(Destination::Public) => {
                let lan_host = {
                    let state = self.0.lock();
                    state
                        .session_of(label)
                        .and_then(|s| state.entries.get(s))
                        .filter(|e| e.mount.as_ref().is_some_and(|m| m.label == label))
                        .and_then(|e| e.mount_lan_host().map(str::to_owned))
                };
                return Ok(name_to_check(&probe, lan_host.as_deref()));
            }
            Ok(Destination::Loopback { port }) => port,
            // A local-network address only from the Lan page the user opened on that host.
            Ok(Destination::Lan) => {
                let state = self.0.lock();
                let own = state.session_of(label).is_some_and(|session| {
                    state.entries.get(session).is_some_and(|e| {
                        e.mount.as_ref().is_some_and(|m| m.label == label)
                            && e.mount_lan_host().is_some()
                            && e.mount_lan_host() == probe.host_str()
                    })
                });
                return if own { Ok(None) } else { Err(()) };
            }
            Err(_) => return Err(()),
        };
        // Loopback (checked synchronously, as on Linux): only from a page of the session's
        // own server (Local mount), never Vibe Work's own ports (fail closed while that list is unknown), only ports
        // of this session or of another session's preview (the cross-session backend).
        let Some(internal) = self.0.internal_ports() else {
            return Err(());
        };
        if internal.contains(&port) {
            return Err(());
        }
        let state = self.0.lock();
        let Some(session) = state.session_of(label) else {
            return Err(());
        };
        let local = state
            .entries
            .get(session)
            .and_then(|e| e.mount.as_ref())
            .is_some_and(|m| m.label == label && m.class == PageClass::Local);
        if local && state.entries.values().any(|e| e.allowed.contains(&port)) {
            Ok(None)
        } else {
            Err(())
        }
    }
}
