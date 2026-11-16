//! Local Network Access for the built-in browser: whether a request of a browser page may reach this computer, the
//! local network or a link-local address, and the person's remembered answer per site.
//!
//! - A PUBLIC page (a page from the web) that makes its own requests there is asked for first; the person answers
//!   Allow or Block for that site (its origin) and the answer is kept in `<storage root>/local-network-sites.json`
//!   until it is changed ([`PreviewService::set_site_answer`]).
//! - A page the person or the agent opened on this computer or the local network is not asked (as before).
//! - The App preview reaches the servers of this chat and of other chats on this computer (every chat's attributed
//!   ports, `State::chat_ports`: the backend in one chat, the frontend in another), never another port, the local
//!   network or link-local addresses.
//! - Vibe Work's own ports are refused before this question (vw-netguard), whatever the answer.
//!
//! A guard listener serves a data store that several mounts share (every session's web pages share one), so a request
//! is attributed to a mount only by its initiator (the request's `Origin`, else its `Referer`): the mounts whose page
//! has that origin. An initiator that is no member's page is a frame of another site: next to a page from the web it
//! passes only when that site itself is allowed. A request that names no initiator counts as a request of every
//! mount on the store: each must be allowed, and the person is asked only when exactly one site is undecided and no
//! other site could have sent it. An App preview that shares its store with an opened local page holds every request
//! it could have sent to the chats' ports: an unattributed request there passes only within the App preview's limits
//! too (the per-webview request barrier on Windows holds it to those ports exactly).
//!
//! Depends on: `state` (mounts, classes, pages), vw-netguard (`LocalRequest`, `IpClass`). Called by: the core's guard
//! context (`vw-core-api::browser_guard`), the request barrier (`hooks::requests`), Debug API site answers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use url::Url;
use vw_netguard::LocalRequest;
use vw_netguard::ip::IpClass;

use crate::service::PreviewService;

/// The file the answers are kept in (inside the preview storage root).
const FILE: &str = "local-network-sites.json";
/// Sites remembered at most (the oldest answers are not dropped: a full list refuses new remembered answers).
pub const MAX_SITES: usize = 2000;

/// The person's answer for one site.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum SiteAnswer {
    /// The site may reach this computer and the local network.
    Allow,
    /// The site may not; it is not asked again.
    Block,
}

/// The answers by site (origin text, `https://example.com`).
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct SiteAnswers {
    #[serde(skip)]
    path: PathBuf,
    sites: BTreeMap<String, SiteAnswer>,
}

impl SiteAnswers {
    /// The saved answers in `root` (none when the file is missing or unreadable).
    pub(crate) fn load(root: &Path) -> Self {
        let path = root.join(FILE);
        let mut answers = std::fs::metadata(&path)
            .ok()
            .filter(|m| m.len() <= 1024 * 1024)
            .and_then(|_| std::fs::read(&path).ok())
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
            .unwrap_or_default();
        answers
            .sites
            .retain(|site, _| site_of_text(site).as_deref() == Some(site.as_str()));
        answers.path = path;
        answers.publish();
        answers
    }

    /// Gives the allowed sites to the web pages' content rules (macOS: loopback literals bypass the guard's proxy).
    fn publish(&self) {
        vw_netguard::rules::set_allowed_sites(
            self.sites
                .iter()
                .filter(|(_, answer)| **answer == SiteAnswer::Allow)
                .map(|(site, _)| site.clone())
                .collect(),
        );
    }

    pub(crate) fn get(&self, site: &str) -> Option<SiteAnswer> {
        self.sites.get(site).copied()
    }

    pub(crate) fn list(&self) -> Vec<(String, SiteAnswer)> {
        self.sites.iter().map(|(s, a)| (s.clone(), *a)).collect()
    }

    /// Sets or forgets `site`'s answer and writes the file (replaced whole; on failure the change is undone).
    pub(crate) fn set(&mut self, site: &str, answer: Option<SiteAnswer>) -> Result<(), String> {
        let before = self.sites.get(site).copied();
        match answer {
            Some(answer) => {
                if before.is_none() && self.sites.len() >= MAX_SITES {
                    return Err("too many sites are remembered".to_owned());
                }
                self.sites.insert(site.to_owned(), answer);
            }
            None => {
                self.sites.remove(site);
            }
        }
        if let Err(error) = self.save() {
            match before {
                Some(old) => self.sites.insert(site.to_owned(), old),
                None => self.sites.remove(site),
            };
            return Err(error);
        }
        self.publish();
        Ok(())
    }

    fn save(&self) -> Result<(), String> {
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }
        let text = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let temp = self.path.with_extension("json.new");
        std::fs::write(&temp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&temp, &self.path).map_err(|e| e.to_string())
    }
}

/// The site (origin text) of an `http`/`https` page address; `None` for anything else.
#[must_use]
pub fn site_of(url: &Url) -> Option<String> {
    matches!(url.scheme(), "http" | "https")
        .then(|| url.origin().ascii_serialization())
        .filter(|o| o != "null")
}

/// [`site_of`] for text (a site the Debug API or a file names).
#[must_use]
pub fn site_of_text(text: &str) -> Option<String> {
    Url::parse(text.trim()).ok().as_ref().and_then(site_of)
}

/// What a request for a non-public target needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalVerdict {
    /// Send it.
    Allow,
    /// Refuse it, with the reason.
    Refuse(String),
    /// Ask the person about `site` (a page of the chat `session`), then decide again.
    Ask {
        /// The site asked about.
        site: String,
        /// The chat (owner session) whose page asks.
        session: String,
    },
}

/// One live mount on the asking store, as the verdict sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Member {
    /// The pinned App preview: its page's site and the ports it may reach (this chat's and other chats' servers).
    App {
        site: Option<String>,
        allowed: Vec<u16>,
    },
    /// A page the person or the agent opened on this computer or the local network.
    Local { site: Option<String> },
    /// A page from the web (its site, `None` for a page without one) of the chat `session`; `navigating`: the
    /// host and port of the top-level navigation it is making now; `loaded`: the document of `site` finished loading
    /// (while it loads, the previous document may still send requests).
    Public {
        site: Option<String>,
        session: String,
        navigating: Option<(String, u16)>,
        loaded: bool,
    },
}

impl Member {
    fn site(&self) -> Option<&str> {
        match self {
            Self::App { site, .. } | Self::Local { site } | Self::Public { site, .. } => {
                site.as_deref()
            }
        }
    }
}

/// How long an admitted top-level navigation counts as the page's own navigation (its connection follows at once).
const NAVIGATION_WINDOW: std::time::Duration = std::time::Duration::from_secs(20);

/// The host (lowercase, no brackets) and port a top-level navigation to `url` connects to, with the time.
pub(crate) fn navigation_target(url: &Url) -> Option<(String, u16, std::time::Instant)> {
    let host = url
        .host_str()?
        .trim_start_matches('[')
        .trim_end_matches(']');
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    Some((
        host,
        url.port_or_known_default()?,
        std::time::Instant::now(),
    ))
}

/// The plain refusal for a blocked site.
#[must_use]
pub fn blocked_text(site: &str) -> String {
    format!(
        "{site} is blocked from reaching apps and devices on this computer or your local network"
    )
}

/// The verdict for `request` on a store whose live mounts are `members` (module docs).
#[cfg(test)]
pub(crate) fn verdict(
    members: &[Member],
    request: &LocalRequest<'_>,
    answer: impl Fn(&str) -> Option<SiteAnswer>,
) -> LocalVerdict {
    verdict_with(members, request, answer).0
}

/// True for a host that is an address literal or `localhost` (no DNS answer can change where it goes).
fn literal_host(host: &str) -> bool {
    host.parse::<std::net::IpAddr>().is_ok() || host == "localhost"
}

/// True when `request` goes to `site` itself and that origin's host is an address literal or `localhost`: the page
/// is then a local page reaching its own origin. A name never counts (a DNS answer could point it at this computer
/// between requests, the rebinding a web page would use), and another port or host is another origin. The request
/// must name the page as its initiator, or name none once the page finished loading: while it loads, the web page it
/// came from may still run and send requests without `Origin` or `Referer`.
fn own_local_origin(site: &str, loaded: bool, request: &LocalRequest<'_>) -> bool {
    let attributed = match request.initiator {
        Some(origin) => origin == site,
        None => loaded,
    };
    if !attributed {
        return false;
    }
    let Ok(url) = Url::parse(site) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    literal_host(&host) && host == request.host && url.port_or_known_default() == Some(request.port)
}

/// [`verdict`], and the index of the member whose top-level navigation the request is (it is used up).
pub(crate) fn verdict_with(
    members: &[Member],
    request: &LocalRequest<'_>,
    answer: impl Fn(&str) -> Option<SiteAnswer>,
) -> (LocalVerdict, Option<usize>) {
    if members.is_empty() {
        return (
            LocalVerdict::Refuse("the page is no longer open".to_owned()),
            None,
        );
    }
    let attributed: Vec<usize> = request
        .initiator
        .map(|origin| {
            (0..members.len())
                .filter(|&i| members[i].site() == Some(origin))
                .collect()
        })
        .unwrap_or_default();
    let known = !attributed.is_empty();
    let indexes: Vec<usize> = if known {
        attributed
    } else {
        (0..members.len()).collect()
    };
    // A top-level navigation is not asked (like Chrome): the one request of a page that navigates itself to exactly
    // this address literal (never a name: a DNS answer could change between the navigation and the request). A
    // request the engine marks as a frame's document is no top-level navigation, even when the engine reported the
    // frame's navigation as the page's (WebKitGTK reports both through one callback): it is asked like the page's
    // other requests.
    if !request.nested
        && literal_host(request.host)
        && let Some(&i) = indexes.iter().find(|&&i| {
            matches!(&members[i], Member::Public { navigating: Some((host, port)), .. }
                if host == request.host && *port == request.port)
        })
    {
        return (LocalVerdict::Allow, Some(i));
    }
    // A request that names a site which is no page on the store comes from a frame of another site inside a page.
    // Next to a page from the web it passes only when that site is allowed AND every page on the store would pass
    // (it never borrows a page's answer, and a blocked page cannot use an allowed frame); nobody is asked (the
    // dialog would name a site that is not a tab). Pages opened on this computer keep their frames.
    if let Some(origin) = request.initiator
        && !known
        && members.iter().any(|m| matches!(m, Member::Public { .. }))
    {
        let pages: Vec<&Member> = members.iter().collect();
        let verdict = match (answer(origin), decide(&pages, false, request, &answer)) {
            (Some(SiteAnswer::Block), _) => LocalVerdict::Refuse(blocked_text(origin)),
            (Some(SiteAnswer::Allow), LocalVerdict::Allow) => LocalVerdict::Allow,
            (_, LocalVerdict::Refuse(reason)) => LocalVerdict::Refuse(reason),
            _ => LocalVerdict::Refuse(format!(
                "a frame of {origin} inside the page sent this request; it reaches this computer or the local \
                 network only when {origin} and the page are allowed"
            )),
        };
        return (verdict, None);
    }
    let requesters: Vec<&Member> = indexes.iter().map(|&i| &members[i]).collect();
    (decide(&requesters, known, request, answer), None)
}

fn decide(
    requesters: &[&Member],
    known: bool,
    request: &LocalRequest<'_>,
    answer: impl Fn(&str) -> Option<SiteAnswer>,
) -> LocalVerdict {
    let mut undecided: Vec<(&str, &str)> = Vec::new();
    for &member in requesters {
        match member {
            Member::Local { .. } => {}
            // An unattributed request on a store the App preview shares with an opened local page may be either
            // page's: it must pass the App preview's limits too (it never borrows the local page's rights).
            Member::App { allowed, .. } => {
                // This computer's own interface address counts as this computer for the question (also behind a
                // name), but the App preview keeps to loopback, where its servers were opened.
                if request.class != IpClass::Loopback || request.own_address {
                    return LocalVerdict::Refuse(
                        "the App preview reaches only the chats' servers on this computer"
                            .to_owned(),
                    );
                }
                if !allowed.contains(&request.port) {
                    return LocalVerdict::Refuse(format!(
                        "port {} is not a server of this chat or another chat (the App preview reaches only those)",
                        request.port
                    ));
                }
            }
            // The page itself is local (a top-level link from a web page led here): its own origin is no web page
            // reaching this computer or the local network.
            Member::Public {
                site: Some(site),
                loaded,
                ..
            } if own_local_origin(site, *loaded, request) => {}
            Member::Public { site: None, .. } => {
                return LocalVerdict::Refuse(
                    "this page has no site that could be allowed to reach this computer or the local network"
                        .to_owned(),
                );
            }
            Member::Public {
                site: Some(site),
                session,
                ..
            } => match answer(site) {
                Some(SiteAnswer::Allow) => {}
                Some(SiteAnswer::Block) => return LocalVerdict::Refuse(blocked_text(site)),
                None => {
                    if !undecided.iter().any(|(s, _)| s == site) {
                        undecided.push((site, session));
                    }
                }
            },
        }
    }
    let sites: std::collections::BTreeSet<&str> =
        requesters.iter().filter_map(|m| m.site()).collect();
    match undecided.as_slice() {
        [] => LocalVerdict::Allow,
        [(site, session)] if known || sites.len() == 1 => LocalVerdict::Ask {
            site: (*site).to_owned(),
            session: (*session).to_owned(),
        },
        _ => LocalVerdict::Refuse(
            "several web pages are open in the browser and this request does not say which one sent it; allow \
             the site first to let it reach this computer or the local network"
                .to_owned(),
        ),
    }
}

impl PreviewService {
    /// The verdict for a guard request on a store whose live mounts are `keys` (webview labels).
    #[must_use]
    pub fn local_access(&self, keys: &[String], request: &LocalRequest<'_>) -> LocalVerdict {
        if self.test_public_target(request) {
            return LocalVerdict::Allow;
        }
        let (labels, mut members): (Vec<String>, Vec<Member>) =
            self.local_members(keys).into_iter().unzip();
        let decide = |members: &[Member]| {
            let answers = self
                .inner
                .site_answers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            verdict_with(members, request, |site| answers.get(site))
        };
        let (verdict, navigation) = decide(&members);
        let Some(i) = navigation else {
            return verdict;
        };
        // A navigation's exemption covers its one request: taken under the lock, so two requests never both use it.
        let taken = {
            let mut state = self.inner.lock();
            let session = labels
                .get(i)
                .and_then(|label| state.session_of(label))
                .map(str::to_owned);
            session
                .and_then(|session| state.entries.get_mut(&session))
                .and_then(|entry| {
                    let same = entry
                        .navigation_target
                        .as_ref()
                        .is_some_and(|(host, port, _)| {
                            host == request.host && *port == request.port
                        });
                    same.then(|| entry.navigation_target.take()).flatten()
                })
                .is_some()
        };
        if taken {
            return verdict;
        }
        if let Some(Member::Public { navigating, .. }) = members.get_mut(i) {
            *navigating = None;
        }
        decide(&members).0
    }

    /// The remembered answer for `site` (origin text).
    #[must_use]
    pub fn site_answer(&self, site: &str) -> Option<SiteAnswer> {
        self.inner
            .site_answers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(site)
    }

    /// Every remembered answer, by site.
    #[must_use]
    pub fn site_answers(&self) -> Vec<(String, SiteAnswer)> {
        self.inner
            .site_answers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .list()
    }

    /// Remembers (`Some`) or forgets (`None`, the site is asked again) the answer for the site of `site`.
    ///
    /// # Errors
    /// `site` is not an `http`/`https` site, or the answers could not be saved.
    pub fn set_site_answer(
        &self,
        site: &str,
        answer: Option<SiteAnswer>,
    ) -> Result<String, String> {
        let site = site_of_text(site).ok_or_else(|| "not an http or https site".to_owned())?;
        self.inner
            .site_answers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set(&site, answer)?;
        tracing::info!(%site, ?answer, "browser local network answer changed");
        Ok(site)
    }

    /// The members of a store, from the live mounts `keys` (labels of closed mounts are left out).
    fn local_members(&self, keys: &[String]) -> Vec<(String, Member)> {
        let internal = self.inner.internal_ports();
        let state = self.inner.lock();
        keys.iter()
            .filter_map(|label| {
                let session = state.session_of(label)?;
                let entry = state.entries.get(session)?;
                let site = entry.url.as_ref().and_then(site_of);
                let pinned_app =
                    state.tabs.contains_key(session) && !state.page_owners.contains_key(session);
                Some((
                    label.clone(),
                    if pinned_app {
                        Member::App {
                            site,
                            allowed: state.chat_ports(session, internal.as_deref()),
                        }
                    } else if entry.mount_class() == crate::policy::PageClass::Web
                        || self.test_public_page(entry.url.as_ref())
                    {
                        Member::Public {
                            site,
                            session: state.owner_of(session),
                            loaded: !entry.loading,
                            navigating: entry
                                .navigation_target
                                .as_ref()
                                .filter(|(_, _, at)| at.elapsed() < NAVIGATION_WINDOW)
                                .map(|(host, port, _)| (host.clone(), *port)),
                        }
                    } else {
                        Member::Local { site }
                    },
                ))
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "local_access_tests.rs"]
mod tests;
