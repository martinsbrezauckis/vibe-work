//! Native current-site cookie access through Tauri's `WebKitGTK`, `WKWebView` and `WebView2` adapters.
//! Runs off the UI thread to avoid `WebView2` cookie-manager deadlocks; never accepts a caller's domain.
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
        let current = webview.cookies_for_url(url.clone()).map_err(fault)?;
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
                let rows = webview.cookies_for_url(query.clone()).map_err(fault)?;
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
            // Include all paths of cookies available to this host; never delete another site's cookies.
            let matches = |c: &tauri::webview::Cookie<'_>| {
                let domain = c.domain().unwrap_or_default().trim_start_matches('.');
                (host == domain || host.ends_with(&format!(".{domain}")))
                    && (clear || (c.name() == name && c.path().unwrap_or("/") == path))
            };
            let all = webview.cookies().map_err(fault)?;
            for cookie in all.into_iter().filter(matches) {
                webview.delete_cookie(cookie).map_err(fault)?;
            }
            let complete = changed(CHANGE_WAIT, || {
                page()?;
                // Verify the same host/path scope we deleted, including non-root paths and secure cookies.
                let rows = webview.cookies().map_err(fault)?;
                page()?;
                Ok(!rows.iter().any(matches))
            })?;
            Ok(if clear {
                json!({"cleared":complete})
            } else {
                json!({"removed":complete,"name":name})
            })
        }
        _ => Err("Unknown cookie operation.".into()),
    }
}

/// The page check uses the preview's tracked mount address, never `Webview::url()` (which can panic on macOS).
fn check_page(hooks: &dyn PreviewHooks, label: &str, url: &Url) -> Result<(), String> {
    if hooks.current_url(label).as_ref() != Some(url) {
        return Err("The page changed; read it again before accessing cookies.".into());
    }
    Ok(())
}
