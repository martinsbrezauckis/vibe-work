//! Sites where page scripts and raw data reads stay off for the rest of this app run. A site is remembered when a
//! page script's result was withheld because the script reached a protected field (`script_reach`), and when a
//! protected field was seen there: by the check before a raw action, or as a redacted field in a page read. Another
//! page of the same site could otherwise load the protected page into a frame, or return a value an earlier script
//! kept in the page's storage. Kept in memory per browser (one per app run), never written to disk; nobody can lift
//! it. `page.read`, typing, clicking and vault sign-in stay available.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use crate::{ActError, Action, Browser};

/// A page script ran and reached a protected field: its result is not returned.
pub const RESULT_WITHHELD: &str = "Result withheld: this page script reached a card number, PIN or password field, so its result is not returned and page scripts and raw data reads are off on this site. Use page.read, page.type, page.click, or auth.fill with a vault login. Protected field values were withheld from this result.";
/// Refused before dispatch on a site where a page script reached a protected field.
pub const NOT_RUN_ON_SITE: &str = "Not run: a page script on this site reached a card number, PIN or password field, so page scripts and raw data reads are off on this site. Use page.read, page.type, page.click, or auth.fill with a vault login. Protected field values were withheld from this result.";
/// Refused before dispatch on a site where a protected field was seen.
pub const NOT_RUN_SITE_HAS_FIELD: &str = "Not run: this site has a card number, PIN or password field, so page scripts and raw data reads are off on this site. Use page.read, page.type, page.click, or auth.fill with a vault login. Protected field values were withheld from this result.";

/// Why a site is remembered. A reached site stays reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Why {
    Seen,
    Reached,
}

#[derive(Clone, Default)]
pub(crate) struct ProtectedOrigins(Arc<Mutex<BTreeMap<String, Why>>>);

impl ProtectedOrigins {
    fn remember(&self, site: String, why: Why) {
        let mut sites = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = sites.entry(site).or_insert(why);
        if why == Why::Reached {
            *entry = Why::Reached;
        }
    }

    fn get(&self, site: &str) -> Option<Why> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(site)
            .copied()
    }
}

/// The key of a page's site: its origin; a page without one (`file:`, `data:`, a sandboxed document) by its own
/// address without query and fragment.
pub(crate) fn site_of(url: &str) -> Option<String> {
    let mut url = url::Url::parse(url).ok()?;
    let origin = url.origin();
    if origin.is_tuple() {
        return Some(origin.ascii_serialization());
    }
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string())
}

/// The actions a remembered site refuses: page scripts and the raw data reads of the protected-field rule.
pub(crate) fn raw_on_site(action: &Action) -> bool {
    match action {
        Action::PageScript(_) => true,
        Action::Network(request) => request.unredacted_headers(),
        _ => matches!(
            action.name(),
            "storage.read" | "cookies.read" | "console.read" | "page.errors" | "network.body"
        ),
    }
}

impl Browser {
    /// The site of `session`'s current page.
    pub(crate) fn page_site(&self, session: &str) -> Option<String> {
        self.service
            .view(session)
            .and_then(|view| view.url)
            .and_then(|url| site_of(&url))
    }

    /// Remembers the site of `session`'s current page.
    pub(crate) fn remember_page_site(&self, session: &str, why: Why) {
        if let Some(site) = self.page_site(session) {
            self.protected_origins.remember(site, why);
        }
    }

    /// Remembers the site of a page address (a read's observed page).
    pub(crate) fn remember_site_of(&self, url: &str, why: Why) {
        if let Some(site) = site_of(url) {
            self.protected_origins.remember(site, why);
        }
    }

    /// Refuses a raw action before dispatch on a remembered site; nothing ran.
    pub(crate) fn refuse_remembered_site(&self, session: &str) -> Result<(), ActError> {
        let Some(why) = self
            .page_site(session)
            .and_then(|site| self.protected_origins.get(&site))
        else {
            return Ok(());
        };
        let mut error = crate::driver::refused(match why {
            Why::Reached => NOT_RUN_ON_SITE,
            Why::Seen => NOT_RUN_SITE_HAS_FIELD,
        });
        error.protected_values_withheld = Some(0);
        Err(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sites_are_origins_and_reached_wins() {
        assert_eq!(
            site_of("https://shop.test:8443/pay?x=1#f").as_deref(),
            Some("https://shop.test:8443")
        );
        assert_eq!(
            site_of("file:///tmp/a.html?q#f").as_deref(),
            Some("file:///tmp/a.html")
        );
        let origins = ProtectedOrigins::default();
        origins.remember("https://a.test".into(), Why::Reached);
        origins.remember("https://a.test".into(), Why::Seen);
        origins.remember("https://b.test".into(), Why::Seen);
        assert_eq!(origins.get("https://a.test"), Some(Why::Reached));
        assert_eq!(origins.get("https://b.test"), Some(Why::Seen));
        origins.remember("https://b.test".into(), Why::Reached);
        assert_eq!(origins.get("https://b.test"), Some(Why::Reached));
        assert_eq!(origins.get("https://c.test"), None);
    }
}
