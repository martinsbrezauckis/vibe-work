//! Exact local-document grants supplied by the core after its chat-folder file checks.
//! Navigation and subrequest hooks compare one selected file; ordinary URL policy never enables the scheme.
use crate::{PreviewError, PreviewService};
use url::Url;

/// Whether both URLs name the same local file (fragment navigation is allowed).
#[must_use]
pub fn same_local_document(approved: &Url, candidate: &Url) -> bool {
    let local = |url: &Url| {
        url.scheme() == "file"
            && url.host_str().is_none()
            && url.query().is_none()
            && url.to_file_path().is_ok()
    };
    if !local(approved) || !local(candidate) {
        return false;
    }
    let mut a = approved.clone();
    let mut b = candidate.clone();
    a.set_fragment(None);
    b.set_fragment(None);
    a == b
}
impl PreviewService {
    /// Grants one browser page access to a local document already checked by the core.
    ///
    /// # Errors
    /// The page is unknown, is an App preview, or the URL is not a local file.
    pub fn allow_local_document(&self, session: &str, url: &Url) -> Result<(), PreviewError> {
        let mut state = self.inner.lock();
        if !same_local_document(url, url) || !state.page_owners.contains_key(session) {
            return Err(PreviewError::InvalidTab("Local files need an owned browser tab and a checked local file URL; open a browser tab first.".into()));
        }
        state.entry(session).local_document = Some(url.clone());
        Ok(())
    }
}
