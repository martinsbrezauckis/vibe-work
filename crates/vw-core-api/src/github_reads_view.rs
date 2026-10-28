//! GitHub reader's public settings contract. Used by Settings and the Debug API; never contains credentials.
use serde::{Deserialize, Serialize};

/// Current availability of the app-owned GitHub reader.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum GithubReadsStatus {
    /// gh is absent from the app's trusted PATH.
    Missing,
    /// gh has no usable login.
    SignedOut,
    /// gh is signed in on github.com.
    SignedIn,
    /// The broker is starting.
    Starting,
    /// The broker is ready.
    On,
    /// The broker could not start.
    Error,
    /// gh is configured for another host.
    Enterprise,
    /// A previously active login disappeared.
    LoggedOut,
}

/// Saved opt-in and live status. Login and reason are display text only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GithubReadsView {
    /// Off by default; no token is stored with this preference.
    pub enabled: bool,
    /// Current state of the reader.
    pub status: GithubReadsStatus,
    /// Signed-in account, when known.
    pub login: Option<String>,
    /// Safe startup failure reason, when known.
    pub reason: Option<String>,
}

/// Read or change the app-wide opt-in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum GithubReadsRequest {
    /// Refresh gh status without returning credentials.
    Read {},
    /// Start or stop reads. The saved preference follows the resulting state.
    Write {
        /// Requested opt-in.
        enabled: bool,
    },
}

#[cfg(feature = "ts")]
pub(crate) fn declarations(config: &ts_rs::Config) -> Vec<String> {
    use ts_rs::TS;
    vec![
        GithubReadsStatus::decl(config),
        GithubReadsView::decl(config),
        GithubReadsRequest::decl(config),
    ]
}
