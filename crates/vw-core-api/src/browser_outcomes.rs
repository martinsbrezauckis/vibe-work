//! Fixed recovery instructions shared by built-in tools and external browser receipts.
//! The caller keeps the outcome code and page provenance; these messages never contain page words.

pub(crate) const SESSION_UNAVAILABLE: &str = "The browser window or session is no longer available. Open a new browser session before trying again.";
pub(crate) const TARGET_UNAVAILABLE: &str = "The browser target is no longer available. List tabs, select a current tab and read the page before acting.";
