//! Native provider-session presence and sign-out for the preview engine.
//! Uses the existing bounded UI-thread dispatch and OS cookie managers; called by platform callers.
//! Only presence and removal results leave the native cookie store.
use super::with_native;
use std::time::Duration;
use tauri::{Webview, Wry};
use vw_preview::EngineError;

/// Reads session-cookie presence through `WebKitGTK`, without exposing cookie values.
#[cfg(target_os = "linux")]
pub(crate) async fn provider_accounts(
    webview: &Webview<Wry>,
) -> Result<Vec<vw_preview::accounts::ProviderAccount>, EngineError> {
    with_native(
        webview,
        "reading provider sessions",
        Duration::from_secs(5),
        move |native, reply| {
            use webkit2gtk::{CookieManagerExt as _, WebViewExt as _, WebsiteDataManagerExt as _};
            let Some(manager) = native
                .inner()
                .website_data_manager()
                .and_then(|data| data.cookie_manager())
            else {
                let _ = reply.send(Err("the cookie manager is unavailable".to_owned()));
                return;
            };
            glib::MainContext::default().spawn_local(async move {
                let mut result = Vec::new();
                for provider in vw_preview::accounts::Provider::ALL {
                    let mut present = false;
                    for uri in provider.cookie_urls() {
                        match manager.cookies_future(uri).await {
                            Ok(cookies) => {
                                present |= cookies.into_iter().any(|mut cookie| {
                                    cookie.name().is_some_and(|name| {
                                        provider
                                            .session_cookie_names()
                                            .iter()
                                            .any(|expected| name.as_str() == *expected)
                                    }) && cookie
                                        .domain()
                                        .is_some_and(|domain| provider.owns_domain(domain.as_str()))
                                });
                            }
                            Err(error) => {
                                let _ = reply
                                    .send(Err(format!("reading provider cookies failed: {error}")));
                                return;
                            }
                        }
                    }
                    result.push(vw_preview::accounts::ProviderAccount { provider, present });
                }
                let _ = reply.send(Ok(result));
            });
        },
    )
    .await
}

/// Removes every enumerated provider domain through `WebKitGTK` and waits until none remains.
#[cfg(target_os = "linux")]
// WORKAROUND(WebKitGTK 2.16): domain enumeration and domain deletion are deprecated but allow
// exact per-provider deletion without reading cookie values. Replace when an equivalent ships.
#[allow(deprecated)]
pub(crate) async fn sign_out_provider(
    webview: &Webview<Wry>,
    provider: vw_preview::accounts::Provider,
) -> Result<(), EngineError> {
    with_native(
        webview,
        "removing provider cookies",
        Duration::from_secs(5),
        move |native, reply| {
            use webkit2gtk::{CookieManagerExt as _, WebViewExt as _, WebsiteDataManagerExt as _};
            let manager = native
                .inner()
                .website_data_manager()
                .and_then(|data| data.cookie_manager())
                .ok_or_else(|| "the cookie manager is unavailable".to_owned());
            let Ok(manager) = manager else {
                let _ = reply.send(Err("the cookie manager is unavailable".to_owned()));
                return;
            };
            glib::MainContext::default().spawn_local(async move {
                for _ in 0..20 {
                    match manager.domains_with_cookies_future().await {
                        Ok(domains) => {
                            let selected: Vec<_> = domains
                                .into_iter()
                                .filter(|domain| provider.owns_domain(domain.as_str()))
                                .collect();
                            if selected.is_empty() {
                                let _ = reply.send(Ok(()));
                                return;
                            }
                            for domain in selected {
                                manager.delete_cookies_for_domain(domain.as_str());
                            }
                        }
                        Err(error) => {
                            let _ =
                                reply.send(Err(format!("listing cookie domains failed: {error}")));
                            return;
                        }
                    }
                    glib::timeout_future(Duration::from_millis(100)).await;
                }
                let result = manager
                    .domains_with_cookies_future()
                    .await
                    .map_err(|error| format!("checking cookie domains failed: {error}"))
                    .and_then(|domains| {
                        if domains
                            .iter()
                            .any(|domain| provider.owns_domain(domain.as_str()))
                        {
                            Err("provider cookies remained after deletion".to_owned())
                        } else {
                            Ok(())
                        }
                    });
                let _ = reply.send(result);
            });
        },
    )
    .await
}

/// Reads `WebView2` cookie metadata from the active shared profile.
#[cfg(windows)]
pub(crate) async fn provider_accounts(
    webview: &Webview<Wry>,
) -> Result<Vec<vw_preview::accounts::ProviderAccount>, EngineError> {
    let cookies = with_native(
        webview,
        "reading provider sessions",
        Duration::from_secs(5),
        move |native, reply| {
            vw_platform::webview::provider_accounts(
                &native.controller(),
                Box::new(move |result| {
                    let _ = reply.send(result);
                }),
            );
        },
    )
    .await?;
    Ok(account_presence(&cookies))
}

/// Removes `WebView2` cookies matching the provider's domains.
#[cfg(windows)]
pub(crate) async fn sign_out_provider(
    webview: &Webview<Wry>,
    provider: vw_preview::accounts::Provider,
) -> Result<(), EngineError> {
    let domains = provider
        .cookie_domains()
        .iter()
        .map(|d| (*d).to_owned())
        .collect();
    with_native(
        webview,
        "removing provider cookies",
        Duration::from_secs(5),
        move |native, reply| {
            vw_platform::webview::sign_out_provider(
                &native.controller(),
                domains,
                Box::new(move |result| {
                    let _ = reply.send(result);
                }),
            );
        },
    )
    .await
}

/// Reduces cookie metadata to presence only.
#[cfg(any(windows, target_os = "macos"))]
fn account_presence(
    cookies: &[vw_platform::webview::CookieIdentity],
) -> Vec<vw_preview::accounts::ProviderAccount> {
    vw_preview::accounts::Provider::ALL
        .into_iter()
        .map(|provider| {
            let present = cookies.iter().any(|cookie| {
                provider.owns_domain(&cookie.domain)
                    && provider
                        .session_cookie_names()
                        .contains(&cookie.name.as_str())
            });
            vw_preview::accounts::ProviderAccount { provider, present }
        })
        .collect()
}

/// Reads `WKWebsiteDataStore` cookie metadata from the active shared profile.
#[cfg(target_os = "macos")]
pub(crate) async fn provider_accounts(
    webview: &Webview<Wry>,
) -> Result<Vec<vw_preview::accounts::ProviderAccount>, EngineError> {
    let cookies = with_native(
        webview,
        "reading provider sessions",
        Duration::from_secs(5),
        move |native, reply| {
            vw_platform::webview::provider_accounts(
                native.inner(),
                Box::new(move |result| {
                    let _ = reply.send(result);
                }),
            );
        },
    )
    .await?;
    Ok(account_presence(&cookies))
}

/// Removes `WKWebsiteDataStore` cookies matching the provider's domains.
#[cfg(target_os = "macos")]
pub(crate) async fn sign_out_provider(
    webview: &Webview<Wry>,
    provider: vw_preview::accounts::Provider,
) -> Result<(), EngineError> {
    let domains = provider
        .cookie_domains()
        .iter()
        .map(|d| (*d).to_owned())
        .collect();
    with_native(
        webview,
        "removing provider cookies",
        Duration::from_secs(5),
        move |native, reply| {
            vw_platform::webview::sign_out_provider(
                native.inner(),
                domains,
                Box::new(move |result| {
                    let _ = reply.send(result);
                }),
            );
        },
    )
    .await
}
