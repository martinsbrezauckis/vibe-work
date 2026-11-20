//! Where the updater may send a request: the package address an update announcement names, and every redirect hop
//! of a request the updater makes (the announcement and the package), must be one of the channel's package origins
//! before that request is sent. `latest.json` is not signed as a whole (only the packages it lists are), so the
//! package address in it is checked here, before the download, not only by the signature after it.
//!
//! - A release channel (no `dangerousInsecureTransportProtocol`): `https` on the default port, no user name or
//!   password, a host name (never an IP literal), and one of Vibe Work's release hosts: `github.com` under the
//!   official repository's releases ([`RELEASES_PATH`]), or one of the hosts GitHub serves release files from after
//!   its redirect ([`RELEASE_ASSET_HOSTS`]). Everything else (other hosts and paths, `http`, IP literals, loopback,
//!   LAN, other ports, user info) is refused.
//! - A test channel (`dangerousInsecureTransportProtocol`, which [`crate::updates::channel`] accepts only with loopback
//!   `http` endpoints and the release bundle scripts refuse for a shipping build): only the origins (scheme, host,
//!   port) of the channel's own loopback endpoints, where the local fake update server of the tests runs. A test
//!   channel never allows the release hosts, and a release channel never allows loopback.
//!
//! The announcement's own endpoint is held to the release rule too: [`crate::updates::channel`] refuses a channel
//! whose `https` endpoint is not on the release server (outside test builds).
//!
//! Depends on: url, `serde_json`, [`crate::updates`]. Called by: `app` (`updater.rs`: before the download and as the
//! redirect policy of the updater's HTTP client), [`crate::updates`], tests.

use serde_json::Value;
use url::{Host, Url};

use crate::updates::channel;

/// The plain failure the person sees when an address is refused (the detail of "The update could not be
/// downloaded."); nothing was requested from that address.
pub const REFUSED: &str =
    "the update address is not Vibe Work's release server, so nothing was downloaded";

/// The path on `github.com` under which the official repository's releases live (`latest.json`, the packages, and
/// the release tags of [`crate::update_view::OFFICIAL_RELEASE_TAGS`]).
pub const RELEASES_PATH: &str = "/martinsbrezauckis/vibe-work/releases/";

/// The hosts `github.com` redirects a release file download to (any path: the file id is opaque).
pub const RELEASE_ASSET_HOSTS: [&str; 2] = [
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
];

/// The error a refused redirect hop ends the request with; [`is_refusal`] finds it in an error's source chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{REFUSED}")]
pub struct OriginRefused;

/// Whether `error` or one of its sources is an [`OriginRefused`].
#[must_use]
pub fn is_refusal(error: &(dyn std::error::Error + 'static)) -> bool {
    std::iter::successors(Some(error), |error| error.source())
        .any(|error| error.downcast_ref::<OriginRefused>().is_some())
}

/// One allowed origin of a test channel.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Origin {
    scheme: String,
    host: Host<String>,
    port: Option<u16>,
}

/// The addresses the updater may request for this channel ([module docs](self)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageOrigins {
    /// `None`: the release hosts. `Some`: a test channel's own loopback origins (possibly none).
    test: Option<Vec<Origin>>,
}

impl PackageOrigins {
    /// Vibe Work's release hosts only.
    #[must_use]
    pub fn release() -> Self {
        Self { test: None }
    }

    /// The origins for the build's `plugins.updater` config: a test channel (accepted with the
    /// `dangerousInsecureTransportProtocol` flag) gets its own loopback endpoint origins, every other config the
    /// release hosts (also a refused or absent one: such a build makes no update request anyway).
    #[must_use]
    pub fn for_channel(config: Option<&Value>) -> Self {
        let insecure = config
            .and_then(|c| c.get("dangerousInsecureTransportProtocol"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let Ok(accepted) = channel(config) else {
            return Self::release();
        };
        if !insecure {
            return Self::release();
        }
        let origins = accepted
            .endpoints
            .iter()
            .filter(|url| is_loopback(url))
            .filter_map(|url| {
                Some(Origin {
                    scheme: url.scheme().to_owned(),
                    host: url.host()?.to_owned(),
                    port: url.port_or_known_default(),
                })
            })
            .collect();
        Self {
            test: Some(origins),
        }
    }

    /// Whether the updater may send a request to `url`.
    #[must_use]
    pub fn allows(&self, url: &Url) -> bool {
        if !url.username().is_empty() || url.password().is_some() {
            return false;
        }
        match &self.test {
            None => is_release_address(url),
            Some(origins) => origins.iter().any(|origin| {
                url.scheme() == origin.scheme
                    && url.host().map(|h| h.to_owned()).as_ref() == Some(&origin.host)
                    && url.port_or_known_default() == origin.port
            }),
        }
    }

    /// `Ok` when [`Self::allows`] `url`, else the plain refusal ([`REFUSED`]).
    ///
    /// # Errors
    /// [`OriginRefused`] for an address outside this channel's origins.
    pub fn check(&self, url: &Url) -> Result<(), OriginRefused> {
        if self.allows(url) {
            Ok(())
        } else {
            Err(OriginRefused)
        }
    }
}

fn is_release_address(url: &Url) -> bool {
    // `Url` drops the scheme's default port, so any port still present is another one.
    if url.scheme() != "https" || url.port().is_some() {
        return false;
    }
    let Some(Host::Domain(host)) = url.host() else {
        return false;
    };
    // Parsed hosts are lower case; a trailing dot or any other spelling is another name and is refused.
    if host == "github.com" {
        return url.path().starts_with(RELEASES_PATH);
    }
    RELEASE_ASSET_HOSTS.contains(&host)
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        Some(Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn the_published_release_addresses_are_allowed() {
        // The 0.1.14 announcement's endpoint, its package addresses, and the hosts github.com redirected them to.
        let release = PackageOrigins::release();
        for text in [
            "https://github.com/martinsbrezauckis/vibe-work/releases/latest/download/latest.json",
            "https://github.com/martinsbrezauckis/vibe-work/releases/download/v0.1.14/latest.json",
            "https://github.com/martinsbrezauckis/vibe-work/releases/download/v0.1.14/Vibe-Work_0.1.14_aarch64.app.tar.gz",
            "https://github.com/martinsbrezauckis/vibe-work/releases/download/v0.1.14/Vibe-Work_0.1.14_amd64.AppImage",
            "https://github.com/martinsbrezauckis/vibe-work/releases/download/v0.1.14/Vibe-Work_0.1.14_x64-setup.exe",
            "https://GitHub.com:443/martinsbrezauckis/vibe-work/releases/download/v0.1.14/x",
            "https://release-assets.githubusercontent.com/github-production-release-asset/1418762906/66451579-03db-453f-9007-32435046383b?sp=r&sig=x",
            "https://objects.githubusercontent.com/github-production-release-asset-2e65be/1/2?X-Amz-Signature=x",
        ] {
            assert!(release.allows(&url(text)), "{text}");
            assert_eq!(release.check(&url(text)), Ok(()));
        }
        // The release tags link and this path agree.
        assert!(
            crate::update_view::OFFICIAL_RELEASE_TAGS
                .starts_with(&format!("https://github.com{RELEASES_PATH}"))
        );
    }

    #[test]
    fn everything_else_is_refused_by_a_release_channel() {
        let release = PackageOrigins::release();
        for text in [
            "http://github.com/martinsbrezauckis/vibe-work/releases/download/v1/x",
            "http://release-assets.githubusercontent.com/a",
            "https://github.com:8443/martinsbrezauckis/vibe-work/releases/download/v1/x",
            "https://release-assets.githubusercontent.com:444/a",
            "https://user@github.com/martinsbrezauckis/vibe-work/releases/download/v1/x",
            "https://user:pw@release-assets.githubusercontent.com/a",
            "https://:pw@release-assets.githubusercontent.com/a",
            "https://github.com/someone/vibe-work/releases/download/v1/x",
            "https://github.com/martinsbrezauckis/vibe-work/archive/x",
            "https://github.com/martinsbrezauckis/vibe-work/releases/../../other/releases/x",
            "https://github.com/martinsbrezauckis/vibe-work/releases/%2e%2e/%2e%2e/x/releases/y",
            "https://github.com./martinsbrezauckis/vibe-work/releases/download/v1/x",
            "https://github.com.example.org/martinsbrezauckis/vibe-work/releases/x",
            "https://evil-githubusercontent.com/a",
            "https://raw.githubusercontent.com/a",
            "https://example.org/x",
            "https://127.0.0.1/x",
            "https://2130706433/x",
            "https://0x7f.1/x",
            "https://[::1]/x",
            "https://[::ffff:127.0.0.1]/x",
            "https://10.0.0.1/x",
            "https://172.16.0.10/x",
            "https://169.254.169.254/latest/meta-data",
            "https://140.82.121.4/martinsbrezauckis/vibe-work/releases/x",
            "https://localhost/x",
            "http://127.0.0.1:4567/pkg",
            "file:///etc/passwd",
            "ftp://github.com/martinsbrezauckis/vibe-work/releases/x",
        ] {
            assert!(!release.allows(&url(text)), "{text}");
            assert_eq!(release.check(&url(text)), Err(OriginRefused), "{text}");
        }
    }

    #[test]
    fn a_test_channel_allows_only_its_own_loopback_origins() {
        let config = json!({
            "pubkey": "k",
            "requireSignedVersion": true,
            "dangerousInsecureTransportProtocol": true,
            "endpoints": ["http://127.0.0.1:4567/latest.json", "http://localhost:80/x"],
        });
        let test = PackageOrigins::for_channel(Some(&config));
        for text in [
            "http://127.0.0.1:4567/pkg",
            "http://127.0.0.1:4567/a/b?c",
            "http://localhost/other",
        ] {
            assert!(test.allows(&url(text)), "{text}");
        }
        for text in [
            "http://127.0.0.1:4568/pkg",
            "https://127.0.0.1:4567/pkg",
            "http://127.0.0.2:4567/pkg",
            "http://[::1]:4567/pkg",
            "http://user@127.0.0.1:4567/pkg",
            "http://172.16.0.10:4567/pkg",
            "https://github.com/martinsbrezauckis/vibe-work/releases/download/v1/x",
            "https://release-assets.githubusercontent.com/a",
        ] {
            assert!(!test.allows(&url(text)), "{text}");
        }
    }

    #[test]
    fn a_channel_without_the_test_flag_gets_the_release_hosts() {
        for config in [
            json!({"pubkey": "k", "requireSignedVersion": true,
                   "endpoints": ["https://github.com/martinsbrezauckis/vibe-work/releases/latest/download/latest.json"]}),
            json!({"pubkey": "k", "requireSignedVersion": true, "dangerousInsecureTransportProtocol": false,
                   "endpoints": ["https://updates.example.invalid/latest.json"]}),
            // Refused channels (an http endpoint without the flag; no pubkey) never become test channels.
            json!({"pubkey": "k", "requireSignedVersion": true, "endpoints": ["http://127.0.0.1:1/x"]}),
            json!({"pubkey": "", "requireSignedVersion": true, "dangerousInsecureTransportProtocol": true,
                   "endpoints": ["http://127.0.0.1:1/x"]}),
            Value::Null,
        ] {
            assert_eq!(
                PackageOrigins::for_channel(Some(&config)),
                PackageOrigins::release()
            );
        }
        assert_eq!(PackageOrigins::for_channel(None), PackageOrigins::release());
        // A test channel's https endpoint adds no origin (not even the release server's).
        let mixed = json!({"pubkey": "k", "requireSignedVersion": true, "dangerousInsecureTransportProtocol": true,
                           "endpoints": ["https://github.com/martinsbrezauckis/vibe-work/releases/latest/download/latest.json"]});
        let test = PackageOrigins::for_channel(Some(&mixed));
        assert!(!test.allows(&url("https://updates.example.invalid/pkg")));
        assert!(!test.allows(&url(
            "https://github.com/martinsbrezauckis/vibe-work/releases/download/v1/x"
        )));
    }

    #[test]
    fn a_refusal_is_found_in_an_error_chain() {
        #[derive(Debug, thiserror::Error)]
        #[error("outer")]
        struct Outer(#[source] OriginRefused);
        assert!(is_refusal(&Outer(OriginRefused)));
        assert!(is_refusal(&OriginRefused));
        assert!(!is_refusal(&std::io::Error::other("x")));
        assert_eq!(OriginRefused.to_string(), REFUSED);
    }
}
