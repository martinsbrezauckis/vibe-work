//! Classification of provider consent pages and the native confirmation boundary.
//!
//! Depends on: `url` for exact HTTPS authority and path matching. Called by: the browser driver and private
//! black-box tests. Invariant: an unknown or mixed permission request is wider access; only an exact basic set
//! may be accepted without the user's own button.

use std::future::Future;
use std::pin::Pin;

use url::Url;

/// A provider with a recognized authorization page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    /// Google account authorization.
    Google,
    /// GitHub authorization.
    GitHub,
    /// Microsoft account authorization.
    Microsoft,
    /// Apple account authorization.
    Apple,
}

impl Provider {
    /// The name shown to the user.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Google => "Google",
            Self::GitHub => "GitHub",
            Self::Microsoft => "Microsoft",
            Self::Apple => "Apple",
        }
    }
}

/// The permission request extracted from one live document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsentPage {
    /// The current document URL.
    pub url: String,
    /// The current page title, clipped by the page probe.
    pub title: String,
    /// Client or service name visible on the page, if available.
    pub client: String,
    /// Scope values from the URL and the page's hidden form inputs.
    pub scopes: Vec<String>,
    /// Whether this is plainly an account picker rather than a consent grant.
    pub account_chooser: bool,
}

/// The classification of a live authorization document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// The URL is outside the exact provider authorization table.
    Other,
    /// An account picker may be used without an access grant.
    AccountChooser(Provider),
    /// Only the provider's listed basic identity scopes are requested.
    Basic(Provider),
    /// An unknown, absent, or wider scope needs native confirmation.
    Wider(Provider),
}

/// Classifies an exact HTTPS authorization page and its requested scopes.
#[must_use]
pub fn classify(page: &ConsentPage) -> Decision {
    let Some(provider) = provider_for_url(&page.url) else {
        return Decision::Other;
    };
    if page.account_chooser {
        return Decision::AccountChooser(provider);
    }
    if page.client.trim().is_empty() || page.scopes.iter().any(|s| s.trim().is_empty()) {
        return Decision::Wider(provider);
    }
    let parts = scope_tokens(&page.scopes);
    if parts.is_empty() {
        return Decision::Wider(provider);
    }
    if parts.iter().all(|scope| basic_scope(provider, scope)) {
        Decision::Basic(provider)
    } else {
        Decision::Wider(provider)
    }
}

/// Splits URL/form scope lists on OAuth's space delimiter and removes exact duplicates for the dialog.
#[must_use]
pub fn scope_tokens(scopes: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    scopes
        .iter()
        .flat_map(|value| {
            value
                .split(|c: char| c.is_ascii_whitespace())
                .filter(|part| !part.is_empty())
        })
        .filter(|part| seen.insert((*part).to_owned()))
        .map(str::to_owned)
        .collect()
}

/// Returns a provider only for its exact HTTPS authorization host and path.
#[must_use]
pub fn provider_for_url(address: &str) -> Option<Provider> {
    let url = Url::parse(address).ok()?;
    if url.scheme() != "https"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    match (url.host_str()?, url.path()) {
        ("accounts.google.com", path)
            if path.starts_with("/o/oauth2/") || path == "/signin/oauth/consent" =>
        {
            Some(Provider::Google)
        }
        ("github.com", "/login/oauth/authorize") => Some(Provider::GitHub),
        ("login.microsoftonline.com", path) if microsoft_path(path) => Some(Provider::Microsoft),
        ("appleid.apple.com", "/auth/authorize") => Some(Provider::Apple),
        _ => None,
    }
}

fn microsoft_path(path: &str) -> bool {
    let mut parts = path.split('/');
    parts.next() == Some("")
        && parts.next().is_some_and(|tenant| !tenant.is_empty())
        && parts.next() == Some("oauth2")
        && parts.next() == Some("v2.0")
        && parts.next() == Some("authorize")
        && parts.next().is_none()
}

fn basic_scope(provider: Provider, scope: &str) -> bool {
    match provider {
        Provider::Google => matches!(scope, "openid" | "email" | "profile"),
        Provider::GitHub => matches!(scope, "read:user" | "user:email"),
        Provider::Microsoft => matches!(scope, "openid" | "profile" | "email" | "User.Read"),
        Provider::Apple => matches!(scope, "name" | "email"),
    }
}

/// A native permission request, bound to the document checked before the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsentRequest {
    /// Session whose browser action requested the grant.
    pub session_id: String,
    /// The automated caller that attempted the action.
    pub actor: crate::Actor,
    /// Provider that owns the page.
    pub provider: Provider,
    /// Page title.
    pub title: String,
    /// Client or service name, if the page supplies one.
    pub client: String,
    /// Complete scopes read from the page.
    pub scopes: Vec<String>,
}

/// A native confirmation response future.
pub type ConsentFuture<'a> = Pin<Box<dyn Future<Output = bool> + Send + 'a>>;

/// The only way to approve a wider provider grant for an automated browser action.
pub trait ConsentConfirmer: Send + Sync {
    /// Asks the user in a native dialog; false means the action must not run.
    fn confirm(&self, request: ConsentRequest) -> ConsentFuture<'_>;
}
