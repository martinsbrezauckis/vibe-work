//! One listener's address permission scope. Browser rights remain tied to its store; service PAC rights are separate.
use super::{Inner, Who};
use crate::decide::{Scope, ScopeFuture};
use std::sync::Arc;

/// The scope of one listener (whose rights it applies).
pub(crate) struct ListenerScope {
    #[cfg(feature = "proxy-transports")]
    pub(crate) system_client: bool,
    pub(crate) inner: Arc<Inner>,
    pub(crate) who: Who,
}

impl Scope for ListenerScope {
    fn system_client_addresses(&self) -> bool {
        #[cfg(feature = "proxy-transports")]
        {
            self.system_client
        }
        #[cfg(not(feature = "proxy-transports"))]
        {
            false
        }
    }

    fn browser_addresses(&self) -> bool {
        let Some(context) = self.inner.context.get() else {
            return false;
        };
        let Who::Store(store, generation) = &self.who else {
            return false;
        };
        let members = self.inner.store_members(store, *generation);
        !members.is_empty() && members.iter().all(|key| context.browser_addresses(key))
    }

    fn loopback(&self, port: u16) -> ScopeFuture<'_> {
        Box::pin(async move {
            if self.inner.is_internal(port) {
                return Err(format!(
                    "port {port} belongs to Vibe Work itself and is never opened"
                ));
            }
            #[cfg(feature = "proxy-transports")]
            if self.system_client {
                return Ok(());
            }
            let nobody = || "servers on this computer are not reachable from this page".to_owned();
            let Some(context) = self.inner.context.get() else {
                return Err(nobody());
            };
            match &self.who {
                Who::Nobody => Err(nobody()),
                Who::Store(store, generation) => {
                    // Every member must have the right (never one mount's right used by another).
                    let members = self.inner.store_members(store, *generation);
                    if members.is_empty() {
                        return Err(nobody());
                    }
                    for key in &members {
                        context.loopback(key, port).await?;
                    }
                    Ok(())
                }
            }
        })
    }

    fn local_access<'a>(&'a self, request: &'a crate::decide::LocalRequest<'a>) -> ScopeFuture<'a> {
        Box::pin(async move {
            // Vibe Work's own ports, for every page whatever the person answered (`0.0.0.0` reaches this computer too).
            if request.class == crate::ip::IpClass::Loopback && self.inner.is_internal(request.port)
            {
                return Err(format!(
                    "port {} belongs to Vibe Work itself and is never opened",
                    request.port
                ));
            }
            #[cfg(feature = "proxy-transports")]
            if self.system_client {
                return Ok(());
            }
            let nobody = || {
                "this computer and the local network are not reachable from this page".to_owned()
            };
            let Some(context) = self.inner.context.get() else {
                return Err(nobody());
            };
            let Who::Store(store, generation) = &self.who else {
                return Err(nobody());
            };
            let members = self.inner.store_members(store, *generation);
            if members.is_empty() {
                return Err(nobody());
            }
            context.local_access(&members, request).await
        })
    }

    fn local_network(&self, host: &str) -> bool {
        let Some(context) = self.inner.context.get() else {
            return false;
        };
        let same = |own: String| own.trim_end_matches('.').eq_ignore_ascii_case(host);
        match &self.who {
            Who::Nobody => false,
            Who::Store(store, generation) => {
                let members = self.inner.store_members(store, *generation);
                !members.is_empty()
                    && members
                        .iter()
                        .all(|key| context.lan_host(key).is_some_and(same))
            }
        }
    }
}
