//! Fixed identity providers whose browser session can be inspected or cleared.
//!
//! Depends on: serde. Called by: preview service, native webview engine and account UI.
//! Invariant: account state contains presence only; cookie values never cross this API.

use serde::{Deserialize, Serialize};

/// An identity provider supported by the account browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    /// Google.
    Google,
    /// GitHub.
    GitHub,
    /// Microsoft.
    Microsoft,
    /// Apple.
    Apple,
}

impl Provider {
    /// The providers in display order.
    pub const ALL: [Self; 4] = [Self::Google, Self::GitHub, Self::Microsoft, Self::Apple];

    /// Fixed sign-in start for a user-controlled navigation (Google uses Gmail's service redirect).
    #[must_use]
    pub const fn sign_in_url(self) -> &'static str {
        match self {
            Self::Google => "https://mail.google.com/",
            Self::GitHub => "https://github.com/login",
            Self::Microsoft => "https://login.live.com/",
            Self::Apple => "https://appleid.apple.com/sign-in",
        }
    }

    /// Cookie host to inspect using the native website data store.
    #[must_use]
    pub const fn cookie_url(self) -> &'static str {
        match self {
            Self::Google => "https://accounts.google.com/",
            Self::GitHub => "https://github.com/",
            Self::Microsoft => "https://login.live.com/",
            Self::Apple => "https://appleid.apple.com/",
        }
    }

    /// Identity hosts queried for session-cookie presence in native stores.
    #[must_use]
    pub const fn cookie_urls(self) -> &'static [&'static str] {
        match self {
            Self::Google => &["https://accounts.google.com/"],
            Self::GitHub => &["https://github.com/"],
            Self::Microsoft => &[
                "https://login.live.com/",
                "https://login.microsoftonline.com/",
            ],
            Self::Apple => &["https://appleid.apple.com/"],
        }
    }

    /// Domains whose cookies belong to this provider for sign-out.
    #[must_use]
    pub const fn cookie_domains(self) -> &'static [&'static str] {
        match self {
            Self::Google => &["google.com", "accounts.google.com"],
            Self::GitHub => &["github.com"],
            Self::Microsoft => &["live.com", "microsoft.com", "microsoftonline.com"],
            Self::Apple => &["apple.com", "appleid.apple.com"],
        }
    }

    /// Expected session-cookie names. Presence is a hint, not proof the remote session is valid.
    #[must_use]
    pub const fn session_cookie_names(self) -> &'static [&'static str] {
        match self {
            Self::Google => &["SID", "HSID", "SSID", "SAPISID", "__Secure-1PSID"],
            Self::GitHub => &["user_session", "__Host-user_session_same_site"],
            Self::Microsoft => &["ESTSAUTH", "ESTSAUTHPERSISTENT", "MSPAuth"],
            Self::Apple => &["myacinfo", "dssid"],
        }
    }

    /// Whether `domain` is in the provider's fixed cookie-domain set.
    #[must_use]
    pub fn owns_domain(self, domain: &str) -> bool {
        let domain = domain.trim_start_matches('.');
        self.cookie_domains().iter().any(|allowed| {
            domain.eq_ignore_ascii_case(allowed)
                || domain
                    .to_ascii_lowercase()
                    .ends_with(&format!(".{allowed}"))
        })
    }
}

/// Session presence for one identity provider; no cookie value or account identity is included.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccount {
    /// Provider.
    pub provider: Provider,
    /// Whether an expected session cookie was found in the native browser store.
    pub present: bool,
}
