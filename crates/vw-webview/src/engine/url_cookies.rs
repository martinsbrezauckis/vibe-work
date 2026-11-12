//! URL-scoped cookie reads shared by site tools and downloads. Runs only on their blocking workers.
//! WK's store returns all cookies; its URL adapter needs a host-aware filter for address literals.
use tauri::{Webview, Wry, webview::Cookie};
use url::Url;

pub(super) fn read(webview: &Webview<Wry>, url: &Url) -> tauri::Result<Vec<Cookie<'static>>> {
    #[cfg(target_os = "macos")]
    return webview.cookies().map(|cookies| {
        cookies
            .into_iter()
            .filter(|cookie| matches_url(cookie, url))
            .collect()
    });
    #[cfg(not(target_os = "macos"))]
    webview.cookies_for_url(url.clone())
}

/// Exact-host rule shared with cookie clear/remove: the domain (leading dot ignored) is this host itself.
/// Parent-domain cookies never match, because the native stores do not say whether they are host-only.
pub(super) fn is_exact_host(domain: &str, host: &str) -> bool {
    domain.trim_start_matches('.').eq_ignore_ascii_case(host)
}

#[cfg(any(target_os = "macos", test))]
fn matches_url(cookie: &Cookie<'_>, url: &Url) -> bool {
    let secure = cookie.secure().unwrap_or_default();
    let host = url.host_str();
    let path = cookie
        .path()
        .filter(|path| path.starts_with('/'))
        .unwrap_or("/");
    let path_matches = url.path() == path
        || url
            .path()
            .strip_prefix(path)
            .is_some_and(|rest| path.ends_with('/') || rest.starts_with('/'));
    // Keep the native adapter's host and secure-cookie policy. Url::domain excludes address
    // literals, whereas NSHTTPCookie domains include them. Path scoping also matters for downloads.
    host.is_some_and(|host| is_exact_host(cookie.domain().unwrap_or_default(), host))
        && path_matches
        && (!secure
            || url.scheme() == "https"
            || (url.scheme() == "http"
                && (host == Some("localhost")
                    || host
                        .and_then(|host| host.parse::<std::net::Ipv4Addr>().ok())
                        .is_some_and(|ip| ip.is_loopback()))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie(domain: &str, path: &str, secure: bool) -> Cookie<'static> {
        Cookie::build(("session", "fixture"))
            .domain(domain.to_owned())
            .path(path.to_owned())
            .secure(secure)
            .build()
    }

    #[test]
    fn loopback_cookie_is_available_to_http_tools_and_downloads() {
        let url = Url::parse("http://127.0.0.1:8080/account").unwrap();
        assert!(matches_url(&cookie("127.0.0.1", "/", false), &url));
        assert!(!matches_url(&cookie("127.0.0.2", "/", false), &url));
    }

    #[test]
    fn loopback_secure_exception_stays_limited_to_local_hosts() {
        for host in ["localhost", "127.0.0.1", "127.0.0.2"] {
            assert!(matches_url(
                &cookie(host, "/", true),
                &Url::parse(&format!("http://{host}/")).unwrap()
            ));
        }
        assert!(!matches_url(
            &cookie("192.0.2.1", "/", true),
            &Url::parse("http://192.0.2.1/").unwrap()
        ));
    }

    #[test]
    fn cookie_path_is_a_directory_boundary_not_an_arbitrary_prefix() {
        let cookie = cookie("example.test", "/account", false);
        for path in ["/account", "/account/", "/account/details"] {
            assert!(matches_url(
                &cookie,
                &Url::parse(&format!("https://example.test{path}")).unwrap()
            ));
        }
        for path in ["/", "/accountant", "/other/account"] {
            assert!(!matches_url(
                &cookie,
                &Url::parse(&format!("https://example.test{path}")).unwrap()
            ));
        }
    }

    #[test]
    fn ordinary_secure_cookie_never_goes_to_http_or_another_host() {
        let cookie = cookie("example.test", "/", true);
        assert!(matches_url(
            &cookie,
            &Url::parse("https://example.test/").unwrap()
        ));
        for url in [
            "http://example.test/",
            "https://other.test/",
            "https://sub.example.test/",
        ] {
            assert!(!matches_url(&cookie, &Url::parse(url).unwrap()));
        }
    }

    #[test]
    fn read_uses_the_same_exact_host_rule_as_clear() {
        let url = Url::parse("https://sub.example.test/").unwrap();
        assert!(matches_url(&cookie(".sub.example.test", "/", true), &url));
        assert!(!matches_url(&cookie("example.test", "/", true), &url));
        assert!(!matches_url(&cookie(".example.test", "/", true), &url));
    }
}
