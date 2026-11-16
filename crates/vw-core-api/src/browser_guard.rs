//! Live browser mount context and network proxy configuration. Backend fetches retain strict scopes.
//!
//! Local Network Access (`local_access`): a request of a browser page for this computer, the local network or a
//! link-local address follows the preview service's verdict (`vw_preview::LocalVerdict`). When the verdict is to ask,
//! the PERSON is asked in the native dialog (never the agent, also for a tab the agent drives): one question per site
//! at a time, every waiting request of that site shares it. The request waits for the answer (nothing is sent before
//! it) at most [`ASK_WAIT`]; Allow or Block is remembered for the site, no answer in time refuses the request and
//! remembers nothing.
use crate::core::Shared;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::sync::watch;
use vw_gate::Confirmer as _;
use vw_netguard::{LocalRequest, PageContext, ScopeFuture};
use vw_platform::sync::LockRecover;
use vw_preview::{LocalVerdict, SiteAnswer};

/// The native dialog method of the Local Network Access question (its buttons are Allow and Block).
pub const LOCAL_NETWORK_DIALOG_METHOD: &str = "vibe-work/localNetworkAccess";
/// Longest wait of a request for the person's answer.
pub(crate) const ASK_WAIT: Duration = Duration::from_secs(60);

/// The answer of one open question: `None` while open, then `Some(None)` for no answer, `Some(Some(a))` for `a`.
type Asked = watch::Receiver<Option<Option<SiteAnswer>>>;

/// Only a currently mounted browser page admits browser destinations; local ones follow Local Network Access.
pub(crate) struct GuardContext {
    shared: Weak<Shared>,
    /// Open questions by site.
    pending: Arc<Mutex<HashMap<String, Asked>>>,
}
impl GuardContext {
    pub(crate) fn new(shared: &Arc<Shared>) -> Self {
        Self {
            shared: Arc::downgrade(shared),
            pending: Arc::default(),
        }
    }

    /// Asks the person about `site` (or joins the open question) and waits for the answer (bounded).
    async fn ask(
        &self,
        shared: &Arc<Shared>,
        site: &str,
        session: &str,
        request: &LocalRequest<'_>,
    ) -> Option<SiteAnswer> {
        let mut answer = {
            let mut pending = self.pending.lock_or_recover();
            if let Some(open) = pending.get(site) {
                open.clone()
            } else {
                let (tell, open) = watch::channel(None);
                pending.insert(site.to_owned(), open.clone());
                let confirmation = question(shared, site, session, request);
                let (shared, pending, site) =
                    (shared.clone(), self.pending.clone(), site.to_owned());
                tokio::spawn(async move {
                    let given =
                        tokio::time::timeout(ASK_WAIT, shared.confirmer.confirm(&confirmation))
                            .await
                            .ok()
                            .map(|c| {
                                if c.approved() {
                                    SiteAnswer::Allow
                                } else {
                                    SiteAnswer::Block
                                }
                            });
                    if let (Some(given), Some(hub)) = (given, &shared.preview) {
                        if let Err(error) = hub.service.set_site_answer(&site, Some(given)) {
                            tracing::warn!(%error, "the local network answer was not remembered");
                        }
                        hub.service.apply_site_answers().await;
                        // An open Settings > Browser list reads the answers again, as after a change made there.
                        shared.emit(crate::event::CoreEvent::BrowserChanged { session_id: None });
                    }
                    pending.lock_or_recover().remove(&site);
                    tell.send_replace(Some(given));
                });
                open
            }
        };
        tokio::time::timeout(
            ASK_WAIT + Duration::from_secs(1),
            answer.wait_for(Option::is_some),
        )
        .await
        .ok()?
        .ok()
        .and_then(|given| (*given).flatten())
    }
}

/// The native question for `site` (plain words, behaviour only).
fn question(
    shared: &Shared,
    site: &str,
    session: &str,
    request: &LocalRequest<'_>,
) -> vw_gate::ConfirmationRequest {
    let place = match request.class {
        vw_netguard::ip::IpClass::Loopback => "this computer".to_owned(),
        _ => format!(
            "{} on your local network",
            vw_gate::dialog_quote(request.host)
        ),
    };
    vw_gate::ConfirmationRequest {
        method: LOCAL_NETWORK_DIALOG_METHOD.to_owned(),
        summary: format!(
            "{} wants to look for and connect to apps and devices on this computer or your local network (it asked \
             for {place}, port {}).\n\nAllow it only if you trust this site. Vibe Work remembers your answer for this \
             site.\n\nChat: {}",
            vw_gate::dialog_quote(site),
            request.port,
            vw_gate::dialog_chat_quote(&shared.names.confirmation_title(&shared.store, session)),
        ),
        params: serde_json::json!({ "sessionId": session, "site": site }),
        // The page asks, not the agent: the person answers, the dialog names the site.
        requester: vw_gate::Requester::Window,
        always: None,
    }
}
impl PageContext for GuardContext {
    fn connected(&self, key: &str, host: &str, port: u16, addresses: &[std::net::SocketAddr]) {
        if let Some(shared) = self.shared.upgrade()
            && let Some(hub) = &shared.preview
        {
            hub.service
                .record_connected_addresses(key, host, port, addresses);
        }
    }

    fn browser_addresses(&self, key: &str) -> bool {
        self.shared
            .upgrade()
            .and_then(|shared| {
                shared
                    .preview
                    .as_ref()
                    .and_then(|hub| hub.service.page_for_mount(key))
            })
            .is_some()
    }
    fn loopback<'a>(&'a self, key: &'a str, _port: u16) -> ScopeFuture<'a> {
        Box::pin(async move {
            if self.browser_addresses(key) {
                Ok(())
            } else {
                Err("the browser mount is no longer active".into())
            }
        })
    }

    fn local_access<'a>(
        &'a self,
        keys: &'a [String],
        request: &'a LocalRequest<'a>,
    ) -> ScopeFuture<'a> {
        Box::pin(async move {
            let closed = || "the browser page is no longer open".to_owned();
            let shared = self.shared.upgrade().ok_or_else(closed)?;
            let hub = shared.preview.clone().ok_or_else(closed)?;
            // Once more after an answer: the answer is remembered, the verdict reads it (and the store may have changed).
            for _ in 0..2 {
                match hub.service.local_access(keys, request) {
                    LocalVerdict::Allow => return Ok(()),
                    LocalVerdict::Refuse(reason) => return Err(reason),
                    LocalVerdict::Ask { site, session } => {
                        if self.ask(&shared, &site, &session, request).await.is_none() {
                            return Err(format!(
                                "{site} may not reach this computer or the local network: the question was not answered"
                            ));
                        }
                    }
                }
            }
            Err(closed())
        })
    }
}

/// The remembered Local Network Access answers (`localNetworkSites`), after a change (`setLocalNetworkSite`: the
/// person in the window; never an agent tool). The Debug API may Block or Forget a site but never Allow one: only the
/// person grants a site access to this computer and the local network (`debug_api`).
pub(crate) async fn sites(
    shared: &Shared,
    request: &crate::browser_view::BrowserRequest,
    debug_api: bool,
) -> Result<crate::browser_view::BrowserResult, crate::error::CoreError> {
    use crate::error::{CoreError, ErrorCode};
    let service = &crate::previews::hub(shared)?.service;
    if let crate::browser_view::BrowserRequest::SetLocalNetworkSite { site, answer } = request {
        if debug_api && *answer == Some(SiteAnswer::Allow) {
            return Err(CoreError::new(
                ErrorCode::Denied,
                "Allowing a site to reach this computer or the local network requires the user. Ask the user to \
                 allow it in Settings > Browser or when the site asks."
                    .to_owned(),
            ));
        }
        service.set_site_answer(site, *answer).map_err(|error| {
            CoreError::new(
                ErrorCode::InvalidParams,
                format!("Local network access: {error}"),
            )
        })?;
        service.apply_site_answers().await;
        shared.emit(crate::event::CoreEvent::BrowserChanged { session_id: None });
    }
    Ok(crate::browser_view::BrowserResult::LocalNetworkSites {
        sites: service
            .site_answers()
            .into_iter()
            .map(|(site, answer)| crate::browser_view::LocalNetworkSite { site, answer })
            .collect(),
    })
}

/// How often the content rules' inputs are compared with the ones the mounted pages hold.
const RULES_RECHECK: Duration = Duration::from_secs(1);
/// How often this computer's own interface addresses are read again (rounds of [`RULES_RECHECK`]).
const OWN_ADDRESSES_EVERY: u32 = 5;

/// Keeps the guard's view of this computer current and the mounted pages' content rules in step with it: reads this
/// computer's own interface addresses at once and every few seconds (`vw_netguard::own`: a page reaching one of them
/// reaches this computer), and installs the mounted pages' lists again whenever an input of the rules changes
/// (`vw_netguard::rules::revision`: the allowed sites, Vibe Work's own ports, those addresses), so a page mounted
/// before a new own port opened or an address appeared holds it too.
pub(crate) fn watch_rule_inputs(shared: &Arc<Shared>) -> tokio::task::JoinHandle<()> {
    let weak = Arc::downgrade(shared);
    let mut applied = vw_netguard::rules::revision();
    tokio::spawn(async move {
        let mut round = 0_u32;
        loop {
            if round.is_multiple_of(OWN_ADDRESSES_EVERY) {
                match tokio::task::spawn_blocking(vw_platform::interface_addresses).await {
                    Ok(Ok(addresses)) => {
                        vw_netguard::own::set_own_addresses(addresses);
                    }
                    // The last list stays: a failed read cannot prove an address went away.
                    Ok(Err(error)) => {
                        tracing::debug!(%error, "this computer's addresses were not read");
                    }
                    Err(error) => tracing::debug!(%error, "the address read did not finish"),
                }
            }
            round = round.wrapping_add(1);
            let Some(shared) = weak.upgrade() else {
                return;
            };
            let Some(hub) = shared.preview.clone() else {
                return;
            };
            drop(shared);
            // Read before installing: a change during the install is installed by the next round.
            let now = vw_netguard::rules::revision();
            if now != applied {
                hub.service.apply_rule_inputs().await;
                applied = now;
            }
            tokio::time::sleep(RULES_RECHECK).await;
        }
    })
}

/// Registers Vibe Work's own ports: refused for every browser page (and strict non-browser scopes). The chats'
/// servers a Local page may reach come from each process scan (`preview_ports`); none before the first one.
pub(crate) fn register_internal_ports(shared: &Shared, guard: &vw_netguard::NetGuard) {
    if let Some(hub) = &shared.preview {
        hub.service
            .set_chat_servers(0, Vec::new(), std::sync::Arc::new(|_| false));
    }
    guard.add_internal_port(shared.vibe_processes.port());
    if let Some(tools) = &shared.agent_tools {
        guard.add_internal_port(tools.endpoint.port());
    }
    if let Some(keys) = &shared.config.keys
        && let Some(port) = url::Url::parse(keys.proxy_base())
            .ok()
            .and_then(|u| u.port_or_known_default())
    {
        guard.add_internal_port(port);
    }
    if let Some(hub) = &shared.preview
        && let Some(port) = hub
            .service
            .policy()
            .dev_origin
            .as_ref()
            .and_then(url::Url::port_or_known_default)
    {
        guard.add_internal_port(port);
    }
}

/// Updates the existing browser guard after vault-backed proxy settings become available.
pub(crate) fn refresh_network_guard(shared: &Shared) {
    let Some(guard) = shared.preview.as_ref().and_then(|hub| hub.guard.as_ref()) else {
        return;
    };
    // Public requests go through the user's proxy: Settings > Network (or the process environment) when
    // set, else the guard keeps the system's proxy settings it read when it was bound.
    #[cfg(not(feature = "proxy-transports"))]
    let network = vw_mistral::network::current();
    #[cfg(not(feature = "proxy-transports"))]
    if network.has_proxy() {
        guard.set_upstream(vw_netguard::Upstream::from_values(
            network.http_proxy.as_deref(),
            network.https_proxy.as_deref(),
            network.all_proxy.as_deref(),
            network.no_proxy.as_deref(),
        ));
    }
    #[cfg(feature = "proxy-transports")]
    {
        let network = vw_mistral::network::guard_current();
        guard.set_upstream(
            vw_netguard::Upstream::from_values(
                network.http_proxy.as_deref(),
                network.https_proxy.as_deref(),
                network.all_proxy.as_deref(),
                network.no_proxy.as_deref(),
            )
            .with_system_resolver()
            .with_certificates(network.guard_certificates()),
        );
    }
}
