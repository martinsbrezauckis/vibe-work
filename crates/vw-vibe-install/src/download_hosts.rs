//! Where the Vibe install may send a request: the uv archive (pinned in `pin/vibe-install.json`), an update's
//! requirements file and the supported-Vibe manifest (both published next to the app's update announcement), and
//! every redirect hop of those requests. A request goes out only when its address passes [`allows`]; otherwise
//! nothing is sent to it.
//!
//! - `https` on the default port, no user name or password, a host name (never an IP literal), and one of the
//!   download servers the pinned addresses use: `github.com` under one of [`GITHUB_RELEASE_PATHS`] (Astral's uv
//!   releases, Vibe Work's own releases), or one of [`RELEASE_ASSET_HOSTS`], the hosts `github.com` redirects a
//!   release file to (any path: the file id is opaque).
//! - `http` only between loopback addresses (the local test servers): a loopback first address, and a redirect from
//!   a loopback address to a loopback address.
//!
//! The rule does not replace the content checks: the uv archive and the requirements file must still match their
//! pinned SHA-256, the manifest its signature.
//!
//! Depends on: url. Called by: [`crate::fetch`].

use url::{Host, Url};

/// The `github.com` paths the downloads live under.
pub const GITHUB_RELEASE_PATHS: [&str; 2] = [
    "/astral-sh/uv/releases/",
    "/martinsbrezauckis/vibe-work/releases/",
];

/// The hosts `github.com` redirects a release file download to.
pub const RELEASE_ASSET_HOSTS: [&str; 2] = [
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
];

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    }
}

fn is_download_server(url: &Url) -> bool {
    // `Url` drops the scheme's default port, so any port still present is another one.
    if url.scheme() != "https" || url.port().is_some() {
        return false;
    }
    let Some(Host::Domain(host)) = url.host() else {
        return false;
    };
    // Parsed hosts are lower case; a trailing dot or any other spelling is another name and is refused.
    if host == "github.com" {
        return GITHUB_RELEASE_PATHS
            .iter()
            .any(|prefix| url.path().starts_with(prefix));
    }
    RELEASE_ASSET_HOSTS.contains(&host)
}

/// Whether a request may go to `to`. `from` is the address that redirected there (`None` for the first request).
#[must_use]
pub fn allows(from: Option<&Url>, to: &Url) -> bool {
    if !to.username().is_empty() || to.password().is_some() {
        return false;
    }
    if to.scheme() == "http" {
        return is_loopback(to) && from.is_none_or(is_loopback);
    }
    is_download_server(to)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn the_pinned_addresses_and_their_recorded_redirects_are_allowed() {
        let pin: serde_json::Value =
            serde_json::from_str(include_str!("../pin/vibe-install.json")).unwrap();
        let assets = pin["uv"]["assets"].as_object().unwrap();
        assert!(!assets.is_empty());
        for asset in assets.values() {
            let text = asset["url"].as_str().unwrap();
            assert!(allows(None, &url(text)), "{text}");
        }
        // The redirect chains the real servers answer (`curl -sIL`): each pinned uv archive 302 -> release-assets; the support manifest
        // latest -> the tag's file (github.com) -> release-assets.
        let github = url(
            "https://github.com/astral-sh/uv/releases/download/0.12.24/uv-x86_64-unknown-linux-gnu.tar.gz",
        );
        let latest = url(
            "https://github.com/martinsbrezauckis/vibe-work/releases/latest/download/vibe-support.json",
        );
        let tagged = url(
            "https://github.com/martinsbrezauckis/vibe-work/releases/download/v0.1.14/vibe-support.json",
        );
        let asset = url(
            "https://release-assets.githubusercontent.com/github-production-release-asset/699532645/cd3695a3-4de6-48aa-9860-219d40398213?sp=r&sig=x",
        );
        assert!(allows(Some(&github), &asset));
        assert!(allows(None, &latest));
        assert!(allows(Some(&latest), &tagged));
        assert!(allows(Some(&tagged), &asset));
        assert!(allows(
            Some(&github),
            &url(
                "https://objects.githubusercontent.com/github-production-release-asset-2e65be/1/2"
            )
        ));
        assert!(allows(
            None,
            &url("https://GitHub.com:443/astral-sh/uv/releases/download/0.12.24/x")
        ));
        // The local test servers.
        assert!(allows(None, &url("http://127.0.0.1:4000/uv.tar.gz")));
        assert!(allows(
            Some(&url("http://127.0.0.1:4000/hop/1")),
            &url("http://127.0.0.1:4000/uv.tar.gz")
        ));
        assert!(allows(None, &url("http://[::1]:4000/x")));
    }

    #[test]
    fn every_other_address_is_refused() {
        let github = url("https://github.com/astral-sh/uv/releases/download/0.12.24/uv.tar.gz");
        for text in [
            "https://example.org/uv.tar.gz",
            "https://github.com.example.org/astral-sh/uv/releases/download/x",
            "https://github.com./astral-sh/uv/releases/download/x",
            "https://github.com/astral-sh/uv/archive/x.tar.gz",
            "https://github.com/someone/uv/releases/download/x",
            "https://github.com/astral-sh/uv-releases/x",
            "https://github.com:8443/astral-sh/uv/releases/download/x",
            "https://release-assets.githubusercontent.com:444/a",
            "https://user@release-assets.githubusercontent.com/a",
            "https://u:p@github.com/astral-sh/uv/releases/download/x",
            "http://github.com/astral-sh/uv/releases/download/x",
            "http://release-assets.githubusercontent.com/a",
            "https://127.0.0.1/x",
            "https://127.0.0.1:4000/x",
            "https://localhost/x",
            "https://[::1]/x",
            "https://140.82.121.4/astral-sh/uv/releases/download/x",
            "https://172.16.0.10/x",
            "http://169.254.169.254/latest/meta-data",
            "http://localhost:4000/x",
            "http://172.16.0.10/x",
            "ftp://github.com/astral-sh/uv/releases/download/x",
            "file:///etc/passwd",
        ] {
            assert!(!allows(None, &url(text)), "first {text}");
            assert!(!allows(Some(&github), &url(text)), "redirect {text}");
        }
        // Plain http only from a loopback address: a release host never redirects to the local machine.
        assert!(!allows(
            Some(&github),
            &url("http://127.0.0.1:4000/uv.tar.gz")
        ));
    }
}
