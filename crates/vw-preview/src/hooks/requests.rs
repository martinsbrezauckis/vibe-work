//! Browser subrequest admission. Depends on live mount identity and supported schemes.
//! The engine resolves local and public destinations under its normal network permissions.
use super::Hooks;
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
        self.0.policy.classify(&probe).map(|_| ()).map_err(|_| ())
    }
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
