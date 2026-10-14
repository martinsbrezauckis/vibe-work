//! The browser arguments that send a `WebView2` (Chromium) webview's traffic through the guard.
//!
//! Chromium's `--proxy-server` carries http, https (`CONNECT`) and WebSocket traffic; by default it skips the proxy
//! for `localhost`, `*.localhost`, `127.0.0.1/8`, `[::1]`, `169.254/16` and `[fe80::]/10`, and the bypass-list entry
//! `<-loopback>` removes that rule (Chromium `net/docs/proxy.md`, "Implicit bypass rules"), so those requests reach
//! the guard too. A WebTransport session is refused by Chromium itself while a proxy applies
//! (`net/quic/dedicated_web_transport_http3_client.cc`: "If a proxy is configured, we fail the connection").
//!
//! Depends on: url. Called by: vw-webview (the engine's browser arguments on Windows), tests.
//! Invariants:
//! - The string starts with wry 0.57's own default arguments (`webview2/mod.rs:300-304`): wry adds them only when the
//!   app passes no arguments of its own, and passing ours replaces them.
//! - The proxy is always a loopback `http://` address with an explicit port (the guard's own listener).

use url::Url;

/// wry 0.57's default `WebView2` arguments (the context menu "mini menu", the PDF UI and `SmartScreen` off).
pub const WRY_DEFAULT_ARGUMENTS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";

/// The `WebView2` browser arguments for webviews whose traffic goes through the guard listener `proxy_url`
/// (`http://127.0.0.1:<port>`): wry's defaults, `--proxy-server` and `--proxy-bypass-list=<-loopback>`.
///
/// Every webview environment that shares a `WebView2` browser process must use identical arguments, so the engine
/// passes the same string (one app-wide listener) to every preview.
///
/// # Errors
/// A reason when `proxy_url` is not `http://127.0.0.1:<port>` or `http://[::1]:<port>`.
pub fn webview2_arguments(proxy_url: &str) -> Result<String, String> {
    let url = Url::parse(proxy_url).map_err(|e| format!("the proxy address is not a URL: {e}"))?;
    let loopback = matches!(
        url.host(),
        Some(url::Host::Ipv4(ip)) if ip.is_loopback()
    ) || matches!(url.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback());
    let Some(port) = url.port() else {
        return Err("the proxy address has no port".to_owned());
    };
    if url.scheme() != "http" || !loopback || url.path() != "/" || url.query().is_some() {
        return Err("the proxy must be http://127.0.0.1:<port>".to_owned());
    }
    let host = url.host_str().unwrap_or("127.0.0.1");
    Ok(format!(
        "{WRY_DEFAULT_ARGUMENTS} --proxy-server=http://{host}:{port} --proxy-bypass-list=<-loopback>"
    ))
}
