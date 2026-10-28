//! Per-platform operations on one preview webview that Tauri has no API for: placing it (Linux needs its own GTK
//! layer), parking a hidden one at a real viewport size, taking a screenshot (Windows: also of a hidden
//! preview, shown outside the window for the capture),
//! securing a fresh `WebView2` webview (its own storage profile, the document-request barrier), and
//! running a closure with the native webview on the UI thread with a bounded wait (ShellX
//! `shellx_browser_webview_runtime.rs:18-60` @ e38c2b5c).
//!
//! Depends on: tauri (`with_webview`), vw-platform (`webview` feature: `WebView2` and `WKWebView` calls that need
//! `unsafe`), gtk/webkit2gtk/cairo on Linux, `linux_layout`. Called by: `engine`.
//! Invariants:
//! - Every wait for the UI thread is bounded; a closure that never answers becomes a timeout, never a hang.
//! - Screenshots are of the preview webview only (never the window: overlapping app UI never leaks into them).
//! - Windows: a webview whose `WebView2` profile is not the session's own is never shown (fail closed), and every
//!   document request it makes is decided by the preview policy before it is sent.

use std::time::Duration;

use tauri::webview::PlatformWebview;
use tauri::{Webview, Wry};
use tokio::sync::oneshot;
use vw_preview::{EngineError, Placement};

/// ShellX Browser's native background sentinel, used only after the logical slot is recorded.
pub(crate) const BROWSER_PARKED_OFFSET: i32 = -30_000;

/// Sends the closure's answer back to the waiting task.
pub(crate) type Reply<T> = oneshot::Sender<Result<T, String>>;

/// Runs `op` with the native webview on the UI thread; `op` answers through its [`Reply`] (now or later, e.g.
/// from a `GLib` future). Waits at most `limit`.
pub(crate) async fn with_native<T: Send + 'static>(
    webview: &Webview<Wry>,
    what: &str,
    limit: Duration,
    op: impl FnOnce(PlatformWebview, Reply<T>) + Send + 'static,
) -> Result<T, EngineError> {
    let (tx, rx) = oneshot::channel();
    webview
        .with_webview(move |platform| op(platform, tx))
        .map_err(|e| EngineError::Platform(format!("{what}: {e}")))?;
    match tokio::time::timeout(limit, rx).await {
        Ok(Ok(Ok(value))) => Ok(value),
        Ok(Ok(Err(reason))) => Err(EngineError::Platform(format!("{what}: {reason}"))),
        Ok(Err(_)) => Err(EngineError::Platform(format!("{what}: no answer"))),
        Err(_) => Err(EngineError::Timeout(what.to_owned())),
    }
}

/// Shows the webview at `placement` (bounds + zoom) or hides it (a hidden webview keeps its size). Async because
/// Linux waits for the UI thread. The geometry is applied BEFORE the webview is shown: a parked webview
/// is never drawn for a frame at its parked spot or size.
#[cfg_attr(not(target_os = "linux"), allow(clippy::unused_async))]
pub(crate) async fn place(
    webview: &Webview<Wry>,
    placement: Option<Placement>,
) -> Result<(), EngineError> {
    let platform = |e: tauri::Error| EngineError::Platform(e.to_string());
    let Some(placement) = placement else {
        return webview.hide().map_err(platform);
    };
    webview.set_zoom(placement.zoom).map_err(platform)?;
    #[cfg(target_os = "linux")]
    {
        let geometry =
            crate::linux_layout::geometry(placement.bounds).map_err(EngineError::Platform)?;
        with_native(
            webview,
            "placing the preview",
            Duration::from_secs(5),
            move |native, reply| {
                let _ = reply.send(crate::linux_layout::show_at(&native.inner(), geometry));
            },
        )
        .await?;
        webview.show().map_err(platform)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let b = placement.bounds.checked().map_err(EngineError::Platform)?;
        let b = crate::page_origin::in_window(
            b,
            crate::page_origin::css_origin(&webview.window()).await?,
        );
        webview
            .set_bounds(tauri::Rect {
                position: tauri::Position::Logical(tauri::LogicalPosition::new(b.x, b.y)),
                size: tauri::Size::Logical(tauri::LogicalSize::new(b.width, b.height)),
            })
            .map_err(platform)?;
        webview.show().map_err(platform)
    }
}

/// Logs native GTK geometry before or after a Browser overlay visibility change.
#[cfg(target_os = "linux")]
pub(crate) async fn trace_browser_overlay(
    webview: &Webview<Wry>,
    hidden: bool,
    after: bool,
) -> Result<(), EngineError> {
    if std::env::var_os("VW_NATIVE_LAYOUT_TRACE").is_none() {
        return Ok(());
    }
    with_native(
        webview,
        "tracing browser overlay geometry",
        Duration::from_secs(5),
        move |native, reply| {
            crate::linux_layout::trace_overlay(&native.inner(), hidden, after);
            let _ = reply.send(Ok(()));
        },
    )
    .await
}

/// The placement a hidden webview gets when the caller gave none (the page checker): the smallest size.
#[must_use]
pub(crate) fn parked_or_smallest(parked: Option<Placement>) -> Placement {
    parked.unwrap_or(Placement {
        bounds: vw_preview::Rect {
            x: 0.0,
            y: 0.0,
            width: vw_preview::MIN_SIDE,
            height: vw_preview::MIN_SIDE,
        },
        zoom: 1.0,
    })
}

/// Where a hidden webview of `size` (logical px) sits on Windows and macOS: entirely outside the window's client area
/// (above and left of it), so even a frame drawn while it is shown there (creation, the hidden capture) can never
/// cover the app's UI. Linux ignores the position (Tauri packs a new webview into its box; `park` then keeps it
/// unmapped in the preview layer).
#[must_use]
pub(crate) fn outside((width, height): (f64, f64)) -> (f64, f64) {
    (-(width + 10_000.0), -(height + 10_000.0))
}

/// Where a hidden webview parked at `bounds` (its would-be placement) sits.
/// - Windows (and the position Linux ignores): [`outside`], above and left of the client area.
/// - macOS: left of the content view at the placement's own top (`bounds.y`, in the content view's coordinates:
///   [`crate::page_origin::in_window`]), never above it. A `WKWebView` whose
///   frame reaches above the window's content layout rect gets an automatic top content inset (the title bar area)
///   measured from its top edge: parked 10,000 pt above the window, its layout viewport height went negative
///   (`innerHeight` -10032 for the vf1 sign-in page; no height media query matched for the viewport fixture) and a
///   vertical scroller took 17 pt of its width (1249 instead of 1280 CSS px at desktop width), so the agent's clicks
///   were refused and a background read differed from the shown page. Left of the window at the
///   shown top it lays out exactly as it does once shown, and still no part of it can be drawn over the app.
#[must_use]
pub(crate) fn outside_at(bounds: vw_preview::Rect) -> (f64, f64) {
    if cfg!(target_os = "macos") {
        (-(bounds.width + 10_000.0), bounds.y)
    } else {
        outside((bounds.width, bounds.height))
    }
}

/// Parks a hidden webview at `placement`'s size and zoom: the page lays out at the viewport it would have
/// on screen, nothing is drawn. Used for a fresh mount that is not shown (`resize_hidden` false: Windows and macOS
/// created it outside the window at this size already) and for a hidden webview whose device width changed
/// (`resize_hidden` true).
/// - Linux: in ONE UI-thread task the widget is put into the preview layer, unmapped (not child-visible), shown and
///   allocated at the size (`linux_layout::park`: GTK ignores allocations of a HIDDEN widget, so it stays visible but
///   unmapped); GTK draws in a later frame, so no frame shows it.
/// - Windows: bounds outside the client area. A webview created at this size needs nothing more; a hidden one being
///   resized is shown there for a moment and hidden again (the renderer takes the size of a visible webview; the
///   same off-screen move as the hidden capture), never inside the window.
/// - macOS: frame outside the content view, hidden (`WKWebView` lays out at its frame while hidden).
#[cfg_attr(not(target_os = "linux"), allow(clippy::unused_async))]
pub(crate) async fn park(
    webview: &Webview<Wry>,
    placement: Placement,
    resize_hidden: bool,
) -> Result<(), EngineError> {
    let platform = |e: tauri::Error| EngineError::Platform(e.to_string());
    let bounds = placement.bounds.checked().map_err(EngineError::Platform)?;
    #[cfg(target_os = "linux")]
    {
        let _ = resize_hidden;
        webview.set_zoom(placement.zoom).map_err(platform)?;
        let (_, _, width, height) =
            crate::linux_layout::geometry(bounds).map_err(EngineError::Platform)?;
        with_native(
            webview,
            "parking the preview",
            Duration::from_secs(5),
            move |native, reply| {
                let _ = reply.send(crate::linux_layout::park(&native.inner(), width, height));
            },
        )
        .await
        // Not `webview.hide()`: a hidden GTK widget gets no allocation (see `linux_layout::park`).
    }
    #[cfg(not(target_os = "linux"))]
    {
        let (x, y) = outside_at(crate::page_origin::in_window(
            bounds,
            crate::page_origin::css_origin(&webview.window()).await?,
        ));
        webview
            .set_bounds(tauri::Rect {
                position: tauri::Position::Logical(tauri::LogicalPosition::new(x, y)),
                size: tauri::Size::Logical(tauri::LogicalSize::new(bounds.width, bounds.height)),
            })
            .map_err(platform)?;
        webview.set_zoom(placement.zoom).map_err(platform)?;
        if cfg!(windows) && resize_hidden {
            webview.show().map_err(platform)?;
        }
        webview.hide().map_err(platform)
    }
}

/// Parks the active Browser child for floating chrome at ShellX's background coordinate.
/// Keep the selected page's actual viewport dimensions and zoom: the 1024 x 768 fallback in
/// ShellX is for a collapsed slot with no usable prior viewport, which this operation rejects.
#[cfg_attr(not(target_os = "linux"), allow(clippy::unused_async))]
pub(crate) async fn park_browser_overlay(
    webview: &Webview<Wry>,
    placement: Placement,
) -> Result<(), EngineError> {
    let platform = |e: tauri::Error| EngineError::Platform(e.to_string());
    let bounds = placement.bounds.checked().map_err(EngineError::Platform)?;
    webview.set_zoom(placement.zoom).map_err(platform)?;
    #[cfg(target_os = "linux")]
    {
        let (_, _, width, height) =
            crate::linux_layout::geometry(bounds).map_err(EngineError::Platform)?;
        with_native(
            webview,
            "parking Browser under chrome",
            Duration::from_secs(5),
            move |native, reply| {
                let _ = reply.send(crate::linux_layout::park_browser_overlay(
                    &native.inner(),
                    width,
                    height,
                ));
            },
        )
        .await
    }
    #[cfg(not(target_os = "linux"))]
    {
        webview.hide().map_err(platform)?;
        // macOS: at the placement's own top, never above the window (see `outside_at`: a frame above the content
        // layout rect gets a top inset larger than its height).
        let top = if cfg!(target_os = "macos") {
            crate::page_origin::in_window(
                bounds,
                crate::page_origin::css_origin(&webview.window()).await?,
            )
            .y
        } else {
            f64::from(BROWSER_PARKED_OFFSET)
        };
        webview
            .set_bounds(tauri::Rect {
                position: tauri::Position::Logical(tauri::LogicalPosition::new(
                    f64::from(BROWSER_PARKED_OFFSET),
                    top,
                )),
                size: tauri::Size::Logical(tauri::LogicalSize::new(bounds.width, bounds.height)),
            })
            .map_err(platform)
    }
}

/// Removes the service-worker registrations of the webview's website data: a worker outlives the
/// page that registered it and could later send requests under a newer mount's rights. Run at every guarded web mount
/// on every platform (Windows and macOS through vw-platform `webview::clear_service_workers`),
/// before the first page; the page environment also refuses new registrations (`page_init.js`) on every platform.
pub(crate) async fn clear_service_workers(
    webview: &Webview<Wry>,
    limit: Duration,
) -> Result<(), EngineError> {
    #[cfg(target_os = "linux")]
    {
        with_native(
            webview,
            "clearing service workers",
            limit,
            |native, reply| {
                use webkit2gtk::{
                    WebViewExt as _, WebsiteDataManagerExtManual as _, WebsiteDataTypes,
                };
                match native.inner().website_data_manager() {
                    Some(manager) => manager.clear(
                        WebsiteDataTypes::SERVICE_WORKER_REGISTRATIONS,
                        webkit2gtk::glib::TimeSpan::from_seconds(0),
                        None::<&webkit2gtk::gio::Cancellable>,
                        move |result| {
                            let _ = reply.send(result.map_err(|e| e.to_string()));
                        },
                    ),
                    None => {
                        let _ = reply.send(Ok(()));
                    }
                }
            },
        )
        .await
    }
    #[cfg(windows)]
    {
        with_native(
            webview,
            "clearing service workers",
            limit,
            |native, reply| {
                vw_platform::webview::clear_service_workers(
                    &native.controller(),
                    Box::new(move |result| {
                        let _ = reply.send(result);
                    }),
                );
            },
        )
        .await
    }
    #[cfg(target_os = "macos")]
    {
        with_native(
            webview,
            "clearing service workers",
            limit,
            |native, reply| {
                vw_platform::webview::clear_service_workers(
                    native.inner(),
                    Box::new(move |result| {
                        let _ = reply.send(result);
                    }),
                );
            },
        )
        .await
    }
}

/// [`clear_service_workers`] for a `guarded` web mount (nothing otherwise). Linux refuses the mount when it fails (as
/// before); Windows and macOS report it and go on: a runtime without the call must not cost the preview (hardening
/// never cuts the feature).
pub(crate) async fn clear_service_workers_at_mount(
    webview: &Webview<Wry>,
    guarded: bool,
) -> Result<(), EngineError> {
    if !guarded {
        return Ok(());
    }
    // WebKitGTK takes about 4.6-4.8 s to clear a storage that holds a live registration (about 30 ms without one): 6 s
    // covers it and keeps the whole mount within the preview service's 10 s engine bound.
    let cleared = clear_service_workers(webview, Duration::from_secs(6)).await;
    if cfg!(target_os = "linux") {
        return cleared;
    }
    if let Err(error) = cleared {
        tracing::warn!(%error, "service workers of the preview storage were not cleared");
    }
    Ok(())
}

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

/// A PNG of what the webview shows, or (`full`: the CSS size, capped by the caller) of the whole document in one render
/// only `WebView2`, through `DevTools` `captureBeyondViewport` with a clip of exactly that size, so the
/// image is bounded on our side (at the preview's page zoom the clip is corrected so the page still fills the image
/// one device pixel per CSS pixel: `vw_platform::webview::full_clip`). Linux and macOS answer `Unsupported` (the caller stitches viewport shots):
/// `WebKitGTK`'s `FullDocument` snapshot sizes itself from the document whatever we ask (real app: a
/// 32,000 x 32,000 px page raised the app's memory peak from 311 MB to 1.72 GB,
/// `.scratch/b16b/falsifier-review-2-oneRender.log`).
pub(crate) async fn screenshot(
    webview: &Webview<Wry>,
    limit: Duration,
    full: Option<(u32, u32)>,
) -> Result<Vec<u8>, EngineError> {
    #[cfg(target_os = "linux")]
    {
        if full.is_some() {
            return Err(EngineError::Unsupported(
                "WebKitGTK's whole-document snapshot is sized by the page, not by us".to_owned(),
            ));
        }
        with_native(
            webview,
            "taking a screenshot",
            limit,
            move |native, reply| {
                use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebViewExt as _};
                let view = native.inner();
                glib::MainContext::default().spawn_local(async move {
                    let result = match view
                        .snapshot_future(SnapshotRegion::Visible, SnapshotOptions::NONE)
                        .await
                    {
                        Ok(surface) => {
                            let mut bytes = Vec::new();
                            surface
                                .write_to_png(&mut bytes)
                                .map(|()| bytes)
                                .map_err(|e| format!("encoding the WebKitGTK snapshot failed: {e}"))
                        }
                        Err(e) => Err(format!("WebKitGTK snapshot failed: {e}")),
                    };
                    let _ = reply.send(result);
                });
            },
        )
        .await
    }
    #[cfg(windows)]
    {
        with_native(
            webview,
            "taking a screenshot",
            limit,
            move |native, reply| {
                vw_platform::webview::capture_png(
                    &native.controller(),
                    full,
                    Box::new(move |result| {
                        let _ = reply.send(result);
                    }),
                );
            },
        )
        .await
    }
    #[cfg(target_os = "macos")]
    {
        if full.is_some() {
            return Err(EngineError::Unsupported(
                "no one-render full-page capture on macOS".to_owned(),
            ));
        }
        with_native(webview, "taking a screenshot", limit, |native, reply| {
            vw_platform::webview::snapshot_png(
                native.inner(),
                Box::new(move |result| {
                    let _ = reply.send(result);
                }),
            );
        })
        .await
    }
}

/// macOS: a sharp viewport snapshot of a zoomed preview (block ): rendered at one point per CSS pixel
/// (`vw_platform::webview::snapshot_png_sharp`); the full-page stitch retries with ordinary slices if it fails.
#[cfg(target_os = "macos")]
pub(crate) async fn screenshot_sharp(
    webview: &Webview<Wry>,
    limit: Duration,
) -> Result<Vec<u8>, EngineError> {
    with_native(
        webview,
        "taking a sharp screenshot",
        limit,
        |native, reply| {
            vw_platform::webview::snapshot_png_sharp(
                native.inner(),
                Box::new(move |result| {
                    let _ = reply.send(result);
                }),
            );
        },
    )
    .await
}

/// macOS: turns WebTransport off for the webview's next documents (`vw_platform::web_transport`): `WebKit` opens
/// WebTransport sessions around the data store's proxy. Call before the first page.
#[cfg(target_os = "macos")]
pub(crate) async fn web_transport_off(webview: &Webview<Wry>) -> Result<(), EngineError> {
    with_native(
        webview,
        "turning WebTransport off",
        Duration::from_secs(4),
        |native, reply| {
            let _ = reply.send(vw_platform::web_transport::turn_off_web_transport(
                native.inner(),
            ));
        },
    )
    .await
}

/// Windows: refuses a webview that did not get the session's own `WebView2` profile (named `profile`, the storage
/// key's hex: the patched tauri-runtime-wry applies the data store identifier as the profile name), then installs
/// the request barrier that asks `decide` for every request (documents and sub-requests; `post` completes a held
/// one on the UI thread). Returns the storage
/// facts (logged by the caller, used for the wipe).
#[cfg(windows)]
pub(crate) async fn secure(
    webview: &Webview<Wry>,
    profile: String,
    decide: vw_platform::webview::DecideRequest,
    post: vw_platform::webview::PostToUi,
) -> Result<vw_platform::webview::StorageFacts, EngineError> {
    with_native(
        webview,
        "securing the preview",
        Duration::from_secs(5),
        move |native, reply| {
            let controller = native.controller();
            let environment = native.environment();
            let result = vw_platform::webview::storage_facts(&controller, &environment).and_then(|facts| {
                if !facts.profile_name.eq_ignore_ascii_case(&profile) {
                    return Err(format!(
                        "the preview did not get its own storage profile (got '{}'); it is not shown, so it \
                         cannot share cookies with another session",
                        facts.profile_name
                    ));
                }
                vw_platform::webview::guard_requests(&controller, &environment, decide, post)?;
                Ok(facts)
            });
            let _ = reply.send(result);
        },
    )
    .await
}

/// Windows: a screenshot of a hidden preview. `WebView2` renders no frames while hidden (Windows 11: no capture path
/// answers until it is shown again), so the webview is moved outside the window's client area (where no part of
/// it is visible), shown, captured and hidden again. `size` is its current size (shown or parked). The caller holds the engine's
/// visibility lock, so a `place` cannot interleave.
#[cfg(windows)]
pub(crate) async fn screenshot_hidden(
    webview: &Webview<Wry>,
    size: (f64, f64),
    limit: Duration,
    full: Option<(u32, u32)>,
) -> Result<Vec<u8>, EngineError> {
    let platform = |e: tauri::Error| EngineError::Platform(e.to_string());
    let (width, height) = size;
    let (x, y) = outside(size);
    webview
        .set_bounds(tauri::Rect {
            position: tauri::Position::Logical(tauri::LogicalPosition::new(x, y)),
            size: tauri::Size::Logical(tauri::LogicalSize::new(width, height)),
        })
        .map_err(platform)?;
    webview.show().map_err(platform)?;
    let shot = screenshot(webview, limit, full).await;
    let hidden = webview.hide().map_err(platform);
    let png = shot?;
    hidden?;
    Ok(png)
}
