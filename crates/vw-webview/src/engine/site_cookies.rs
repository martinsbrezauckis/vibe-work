//! Native current-site cookie access through Tauri's `WebKitGTK`, `WKWebView` and `WebView2` adapters.
//! Runs off the UI thread to avoid `WebView2` cookie-manager deadlocks; never accepts a caller's domain.
//! Depends on: platform cookie managers and current-document verification.
//! Called by: preview engine site-cookie requests.
use super::cookie_verification::{CHANGE_WAIT, changed};
use serde_json::{Value, json};
use std::sync::Arc;
use tauri::{Webview, Wry};
use url::Url;
use vw_preview::{CookieRequest, EngineError, PreviewHooks};

pub(super) async fn run(
    webview: Webview<Wry>,
    hooks: Arc<dyn PreviewHooks>,
    label: String,
    url: Url,
    request: CookieRequest,
) -> Result<Value, EngineError> {
    tokio::task::spawn_blocking(move || {
        let page = || check_page(hooks.as_ref(), &label, &url);
        page()?;
        cookies(&webview, &url, &request, &page)
    })
    .await
    .map_err(|_| EngineError::Platform("Cookie operation failed.".into()))?
    .map_err(EngineError::Platform)
}

fn cookies(
    webview: &Webview<Wry>,
    url: &Url,
    request: &CookieRequest,
    page: &dyn Fn() -> Result<(), String>,
) -> Result<Value, String> {
    let args = request.parameters();
    let fault = |_| "Native cookie access failed; check this site before trying again.".to_owned();
    let host = url.host_str().ok_or("Cookies need a current site.")?;
    if request.name() != "cookies.change" {
        let current = super::url_cookies::read(webview, url).map_err(fault)?;
        let names = args["names"].as_array();
        let mut rows = Vec::new();
        for cookie in current
            .iter()
            .filter(|c| {
                names.is_none_or(|names| names.iter().any(|n| n.as_str() == Some(c.name())))
            })
            .take(100)
        {
            let mut row = json!({"name":cookie.name(),"domain":cookie.domain(),"path":cookie.path(),"secure":cookie.secure().unwrap_or(false),"httpOnly":cookie.http_only().unwrap_or(false),"sameSite":cookie.same_site().map(|s|s.to_string()),"expirationDate":cookie.expires_datetime().map(tauri::webview::cookie::time::OffsetDateTime::unix_timestamp)});
            if request.name() == "cookies.read" {
                row["value"] = json!(cookie.value());
            }
            rows.push(row);
        }
        return Ok(json!({"cookies":rows}));
    }
    let name = args["name"].as_str().unwrap_or_default();
    let path = args["path"].as_str().unwrap_or("/");
    match args["operation"].as_str() {
        Some("set") => {
            let mut cookie = tauri::webview::Cookie::new(
                name.to_owned(),
                args["value"].as_str().unwrap_or_default().to_owned(),
            );
            cookie.set_domain(host.to_owned());
            cookie.set_path(path.to_owned());
            cookie.set_secure(args["secure"] == true);
            cookie.set_http_only(args["httpOnly"] == true);
            // Cookie::parse supplies the crate's SameSite type without tying this adapter to another cookie version.
            if let Some(same) = args["sameSite"].as_str().filter(|s| *s != "unspecified") {
                let same = match same {
                    "no_restriction" => "None",
                    "strict" => "Strict",
                    _ => "Lax",
                };
                if let Ok(parsed) = tauri::webview::Cookie::parse(format!("n=v; SameSite={same}")) {
                    cookie.set_same_site(parsed.same_site());
                }
            }
            webview.set_cookie(cookie).map_err(fault)?;
            let mut query = url.clone();
            query.set_path(path);
            let set = changed(CHANGE_WAIT, || {
                page()?;
                let rows = super::url_cookies::read(webview, &query).map_err(fault)?;
                page()?;
                Ok(rows.iter().any(|c| {
                    c.name() == name
                        && c.value() == args["value"].as_str().unwrap_or_default()
                        && c.path().unwrap_or("/") == path
                }))
            })?;
            Ok(json!({"set":set,"name":name}))
        }
        Some("remove" | "clear") => {
            let clear = args["operation"] == "clear";
            let wanted = |c: &tauri::webview::Cookie<'_>| {
                clear || (c.name() == name && c.path().unwrap_or("/") == path)
            };
            // Delete only cookies whose domain is exactly this host. The native stores do not say whether a
            // parent-domain cookie is host-only (unavailable to this host), so those are kept and reported.
            let exact = |c: &tauri::webview::Cookie<'_>| {
                scope(host, c.domain().unwrap_or_default()) == Scope::Exact && wanted(c)
            };
            let all = webview.cookies().map_err(fault)?;
            let kept = all
                .iter()
                .filter(|c| {
                    scope(host, c.domain().unwrap_or_default()) == Scope::Parent && wanted(c)
                })
                .count();
            for cookie in all.into_iter().filter(|c| exact(c)) {
                webview.delete_cookie(cookie).map_err(fault)?;
            }
            let deleted = changed(CHANGE_WAIT, || {
                page()?;
                // Verify the same exact-host scope we deleted, including non-root paths and secure cookies.
                let rows = webview.cookies().map_err(fault)?;
                page()?;
                Ok(!rows.iter().any(&exact))
            })?;
            Ok(change_result(clear, name, deleted, kept))
        }
        _ => Err("Unknown cookie operation.".into()),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Scope {
    /// The cookie's domain is this host (with or without a leading dot).
    Exact,
    /// A parent domain's cookie, possibly host-only for that parent site; never deleted from here.
    Parent,
    Other,
}

fn scope(host: &str, domain: &str) -> Scope {
    let parent = domain.trim_start_matches('.').to_ascii_lowercase();
    if super::url_cookies::is_exact_host(domain, host) {
        Scope::Exact
    } else if !parent.is_empty() && host.to_ascii_lowercase().ends_with(&format!(".{parent}")) {
        Scope::Parent
    } else {
        Scope::Other
    }
}

/// `cleared`/`removed` is true only when nothing matching is left that this site may use: kept parent-domain
/// cookies make the result incomplete, and the note says so in plain words for the agent.
fn change_result(clear: bool, name: &str, deleted: bool, kept: usize) -> Value {
    let done = deleted && kept == 0;
    let mut out = if clear {
        json!({"cleared":done})
    } else {
        json!({"removed":done,"name":name})
    };
    if kept > 0 {
        out["complete"] = json!(false);
        out["parentDomainCookiesKept"] = json!(kept);
        out["note"] = json!(format!(
            "{kept} cookie(s) set for a parent domain were not deleted: they may belong only to that parent site. \
             Open the parent site to clear them there."
        ));
    } else if !deleted {
        out["complete"] = json!(false);
    }
    out
}

/// The page check uses the preview's tracked mount address, never `Webview::url()` (which can panic on macOS).
fn check_page(hooks: &dyn PreviewHooks, label: &str, url: &Url) -> Result<(), String> {
    if hooks.current_url(label).as_ref() != Some(url) {
        return Err("The page changed; read it again before accessing cookies.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exact_host_cookies_are_in_delete_scope() {
        let host = "sub.example.test";
        assert_eq!(scope(host, "sub.example.test"), Scope::Exact);
        assert_eq!(scope(host, ".sub.example.test"), Scope::Exact);
        assert_eq!(scope(host, "SUB.example.test"), Scope::Exact);
        // A host-only cookie of the parent site and a parent domain cookie look the same in the native store;
        // neither is deleted from the subdomain.
        assert_eq!(scope(host, "example.test"), Scope::Parent);
        assert_eq!(scope(host, ".example.test"), Scope::Parent);
        for other in [
            "other.test",
            "xsub.example.test",
            "deep.sub.example.test",
            "ample.test",
            "",
            ".",
        ] {
            assert_eq!(scope(host, other), Scope::Other, "{other}");
        }
    }

    #[test]
    fn kept_parent_cookies_make_the_result_incomplete() {
        let clean = change_result(true, "", true, 0);
        assert_eq!(clean, json!({"cleared":true}));
        let kept = change_result(true, "", true, 2);
        assert_eq!(kept["cleared"], false);
        assert_eq!(kept["complete"], false);
        assert_eq!(kept["parentDomainCookiesKept"], 2);
        assert!(kept["note"].as_str().unwrap().contains("not deleted"));
        let removed = change_result(false, "sid", true, 1);
        assert_eq!(removed["removed"], false);
        assert_eq!(removed["name"], "sid");
        assert_eq!(change_result(false, "sid", false, 0)["complete"], false);
    }
}
