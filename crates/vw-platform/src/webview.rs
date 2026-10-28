//! Native webview calls the preview engine (vw-webview) needs and Tauri does not offer, each behind a
//! safe function: a PNG of a `WebView2` webview (`DevTools` Protocol `Page.captureScreenshot`, the path ShellX
//! uses on Windows), a PNG of a `WKWebView`
//! (`takeSnapshotWithConfiguration:completionHandler:`; a sharp one of a zoomed preview in `sharp_mac`, sized by
//! [`sharp`]), and the macOS major version (identified data stores need macOS 14). For `WebView2`, this adds: what storage a webview really got ([`StorageFacts`]: the effective
//! user data folder and profile, so the engine can refuse a preview without its own profile), and a request
//! barrier for document requests ([`guard_requests`]): `WebView2` sends the GET of a navigation the
//! host cancels in `NavigationStarting` (Microsoft Learn, `ICoreWebView2NavigationStartingEventArgs::put_Cancel`:
//! "GET HTTP requests may happen, while the host is responding"; reproduced on Windows 11 for a direct navigation and a
//! 302), so the refusal must happen in `WebResourceRequested`, which answers before the request is sent. The barrier
//! also covers every other request of the webview (scripts, images, `fetch`, frames' sub-requests):
//! Chromium sends loopback traffic around a proxy, so the agent browser's loopback and private-network rule is
//! decided here per webview (its mount and page class) on Windows.
//!
//! Feature `webview` (off by default, so crates that do not show webviews never compile these bindings).
//! Depends on: `webview2-com` + windows (Windows), objc2 / objc2-web-kit / objc2-app-kit / block2 (macOS), base64 and
//! `serde_json` (the `DevTools` answer). Called by: vw-webview `platform` (inside Tauri's `with_webview`, on the UI
//! thread).
//! Invariants:
//! - Every function answers through its callback exactly once (also on failure), so a waiting task never hangs.
//! - `unsafe` is confined to the platform calls, each with its SAFETY comment.
//!
//! For `WebView2`, this also reports: whether a setting puts every environment of the app into one shared user data
//! folder, and which ([`user_data_folder_override`]: the environments then share one browser process and must use
//! identical browser arguments). The macOS content rules live in `content_rules`.

mod full_clip;
mod pdf;
pub mod sharp;
mod site_data;
pub use pdf::PdfFile;
pub use site_data::ClearDone;
#[cfg(any(windows, target_os = "macos"))]
pub use site_data::clear_service_workers;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod sharp_mac;
#[cfg(target_os = "macos")]
pub use sharp_mac::snapshot_png_sharp;

#[cfg(windows)]
#[allow(unsafe_code)]
mod pdf_windows;
#[cfg(windows)]
pub use pdf_windows::print_pdf;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod pdf_macos;
#[cfg(target_os = "macos")]
pub use pdf_macos::print_pdf;

pub use full_clip::{FullClip, full_clip};

/// A completion callback carrying the PNG or a text reason.
pub type PngDone = Box<dyn FnOnce(Result<Vec<u8>, String>) + Send>;

/// The storage `WebView2` gave one webview, as `WebView2` itself reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageFacts {
    /// The environment's effective user data folder (`WEBVIEW2_USER_DATA_FOLDER` or the `UserDataFolder` policy
    /// replace the one the app asked for).
    pub user_data_folder: String,
    /// The profile's name (empty for the default profile).
    pub profile_name: String,
    /// The profile's folder: where its cookies and site data live.
    pub profile_path: String,
}

/// Cookie name and domain only; native account inspection never exposes values.
#[derive(Clone, Debug)]
pub struct CookieIdentity {
    /// Name used to detect a session cookie.
    pub name: String,
    /// Domain used to scope sign-out.
    pub domain: String,
}

/// A request barrier's answer (a host-name lookup never blocks the UI thread).
pub enum Verdict {
    /// Send the request.
    Allow,
    /// Answer `403` without sending it.
    Deny,
    /// Decided later: called with the callback that receives the verdict (`true`: send it) from any thread; the
    /// request is held meanwhile (a `WebView2` deferral, completed back on the UI thread through [`PostToUi`]).
    Later(LaterVerdict),
}

/// Receives a held request's verdict (`true`: send it).
pub type VerdictDone = Box<dyn FnOnce(bool) + Send>;

/// Registers the callback a held request's verdict goes to.
pub type LaterVerdict = Box<dyn FnOnce(VerdictDone) + Send>;

/// Decides a request: its URL as text and whether it is a document request (main frame or frame). Called on the UI
/// thread; must return at once.
pub type DecideRequest = Box<dyn Fn(&str, bool) -> Verdict + Send + Sync>;

/// Runs a closure on the UI thread (the app's main thread, which owns the `WebView2` objects).
pub type PostToUi = std::sync::Arc<dyn Fn(Box<dyn FnOnce() + Send>) + Send + Sync>;

#[cfg(windows)]
pub use windows_impl::{
    arguments_change_proxy_at, capture_png, guard_requests, policy_arguments_change_proxy,
    policy_names_folder_at, provider_accounts, shared_user_data_folder, sign_out_provider,
    storage_facts, user_data_folder_override,
};

/// What puts every `WebView2` environment of the app into one user data folder (Windows,
/// `user_data_folder_override`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderOverride {
    /// The `WEBVIEW2_USER_DATA_FOLDER` environment variable.
    Variable,
    /// A `UserDataFolder` policy value for this app (machine or current user).
    Policy,
}

#[cfg(target_os = "macos")]
pub use macos_impl::{provider_accounts, sign_out_provider, snapshot_png};

/// The macOS major version ([`crate::system_version`]), `None` when it cannot be read. Identified website data
/// stores need macOS 14 (the preview storage falls back below it).
#[cfg(target_os = "macos")]
#[must_use]
pub fn macos_major_version() -> Option<u32> {
    crate::system_version()?.1.split('.').next()?.parse().ok()
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_impl {
    use std::sync::{Arc, Mutex, PoisonError};

    use base64::Engine as _;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT,
        COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL, ICoreWebView2_2, ICoreWebView2_13,
        ICoreWebView2_22, ICoreWebView2Controller, ICoreWebView2Cookie, ICoreWebView2CookieList,
        ICoreWebView2CookieManager, ICoreWebView2Deferral, ICoreWebView2Environment,
        ICoreWebView2Environment7, ICoreWebView2WebResourceRequestedEventArgs,
    };
    use webview2_com::{
        CallDevToolsProtocolMethodCompletedHandler, GetCookiesCompletedHandler,
        WebResourceRequestedEventHandler, take_pwstr,
    };
    use windows::core::{HSTRING, Interface, PWSTR, w};

    use super::CookieIdentity;
    use super::{DecideRequest, PngDone, PostToUi, StorageFacts, Verdict, full_clip};

    /// Reads only cookie names and domains from a `WebView2` cookie list.
    fn cookie_identities(list: &ICoreWebView2CookieList) -> Result<Vec<CookieIdentity>, String> {
        // SAFETY: the list belongs to the live WebView2 callback on the UI thread; getters allocate strings
        // released by `take_pwstr`. Values are never requested.
        unsafe {
            let mut count = 0;
            list.Count(&raw mut count)
                .map_err(|e| format!("cookie count: {e}"))?;
            let mut result = Vec::new();
            for index in 0..count {
                let cookie = list
                    .GetValueAtIndex(index)
                    .map_err(|e| format!("cookie index: {e}"))?;
                let mut name = PWSTR::null();
                let mut domain = PWSTR::null();
                cookie
                    .Name(&raw mut name)
                    .map_err(|e| format!("cookie name: {e}"))?;
                cookie
                    .Domain(&raw mut domain)
                    .map_err(|e| format!("cookie domain: {e}"))?;
                result.push(CookieIdentity {
                    name: take_pwstr(name),
                    domain: take_pwstr(domain),
                });
            }
            Ok(result)
        }
    }

    /// Inspects cookie metadata in the native `WebView2` store (call from Tauri's UI thread).
    pub fn provider_accounts(
        controller: &ICoreWebView2Controller,
        done: Box<dyn FnOnce(Result<Vec<CookieIdentity>, String>) + Send>,
    ) {
        let done = Arc::new(Mutex::new(Some(done)));
        let callback_done = done.clone();
        let handler = GetCookiesCompletedHandler::create(Box::new(move |status, list| {
            let result = status
                .map_err(|e| format!("cookie query failed: {e}"))
                .and_then(|()| list.ok_or_else(|| "cookie list is unavailable".to_owned()))
                .and_then(|list| cookie_identities(&list));
            if let Some(done) = callback_done
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
            {
                done(result);
            }
            Ok(())
        }));
        // SAFETY: the live controller is supplied by Tauri on the UI thread. Empty URI requests all cookies;
        // the callback receives the list on that thread and reads metadata only.
        let started = unsafe {
            controller
                .CoreWebView2()
                .and_then(|core| core.cast::<ICoreWebView2_2>())
                .and_then(|core| core.CookieManager())
                .and_then(|manager| manager.GetCookies(w!(""), &handler))
        };
        if let Err(error) = started
            && let Some(done) = done.lock().unwrap_or_else(PoisonError::into_inner).take()
        {
            done(Err(format!("cookie query could not start: {error}")));
        }
    }

    /// Deletes all cookies whose domain matches one of `domains` (including subdomains), by native cookie handle.
    pub fn sign_out_provider(
        controller: &ICoreWebView2Controller,
        domains: Vec<String>,
        done: Box<dyn FnOnce(Result<(), String>) + Send>,
    ) {
        let done = Arc::new(Mutex::new(Some(done)));
        let callback_done = done.clone();
        // SAFETY: the live controller is supplied by Tauri on the UI thread; the cookie manager and list are used
        // only on that thread. The callback never reads values.
        let manager = unsafe {
            controller
                .CoreWebView2()
                .and_then(|core| core.cast::<ICoreWebView2_2>())
                .and_then(|core| core.CookieManager())
        };
        let Ok(manager) = manager else {
            if let Some(done) = done.lock().unwrap_or_else(PoisonError::into_inner).take() {
                done(Err("the cookie manager is unavailable".to_owned()));
            }
            return;
        };
        let for_callback = manager.clone();
        let handler = GetCookiesCompletedHandler::create(Box::new(move |status, list| {
            let result = status
                .map_err(|e| format!("cookie query failed: {e}"))
                .and_then(|()| list.ok_or_else(|| "cookie list is unavailable".to_owned()))
                .and_then(|list| delete_provider_cookies(&for_callback, &list, &domains));
            if let Some(done) = callback_done
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
            {
                done(result);
            }
            Ok(())
        }));
        // SAFETY: an empty URI queries all cookies in this profile; the callback filters by fixed domains.
        let started = unsafe { manager.GetCookies(w!(""), &handler) };
        if let Err(error) = started
            && let Some(done) = done.lock().unwrap_or_else(PoisonError::into_inner).take()
        {
            done(Err(format!("cookie deletion could not start: {error}")));
        }
    }

    fn delete_provider_cookies(
        manager: &ICoreWebView2CookieManager,
        list: &ICoreWebView2CookieList,
        domains: &[String],
    ) -> Result<(), String> {
        // SAFETY: the manager and list belong to the live WebView2 callback on the UI thread; each getter's
        // string is freed by `take_pwstr`. We delete the native cookie object, without reading its value.
        unsafe {
            let mut count = 0;
            list.Count(&raw mut count)
                .map_err(|e| format!("cookie count: {e}"))?;
            for index in 0..count {
                let cookie: ICoreWebView2Cookie = list
                    .GetValueAtIndex(index)
                    .map_err(|e| format!("cookie index: {e}"))?;
                let mut domain = PWSTR::null();
                cookie
                    .Domain(&raw mut domain)
                    .map_err(|e| format!("cookie domain: {e}"))?;
                let domain = take_pwstr(domain);
                let domain = domain.trim_start_matches('.');
                if domains.iter().any(|allowed| {
                    domain.eq_ignore_ascii_case(allowed)
                        || domain
                            .to_ascii_lowercase()
                            .ends_with(&format!(".{allowed}"))
                }) {
                    manager
                        .DeleteCookie(&cookie)
                        .map_err(|e| format!("deleting provider cookie: {e}"))?;
                }
            }
            Ok(())
        }
    }

    /// What replaces the user data folder of every `WebView2` environment of this app with one folder (Microsoft
    /// Learn, `CreateCoreWebView2EnvironmentWithOptions` and the Win32 globals page, 2026-09): the
    /// `WEBVIEW2_USER_DATA_FOLDER` variable (non-empty), else a `UserDataFolder` policy value for this app under
    /// `Software\Policies\Microsoft\Edge\WebView2\UserDataFolder` in the machine's or the current user's
    /// registry. `WebView2` looks that value up by the process's Application User Model ID, then by the exe name;
    /// a `*` value "cannot be used for the userDataFolder option". Vibe Work sets no explicit `AppUserModelID`,
    /// so the value named after its exe file (registry names ignore case) is the one that applies; a value for
    /// another app is not this app's (any value under the key used to count). A setting missed here still
    /// cannot give a preview other arguments: `WebView2` then refuses the preview's creation
    /// (`ERROR_INVALID_STATE`), which the engine handles the same way. Unreadable keys count as absent.
    #[must_use]
    pub fn user_data_folder_override() -> Option<super::FolderOverride> {
        if std::env::var_os("WEBVIEW2_USER_DATA_FOLDER").is_some_and(|v| !v.is_empty()) {
            return Some(super::FolderOverride::Variable);
        }
        let exe = std::env::current_exe().ok()?;
        let exe = exe.file_name()?.to_str()?;
        [
            windows_registry::LOCAL_MACHINE,
            windows_registry::CURRENT_USER,
        ]
        .into_iter()
        .any(|root| policy_names_folder(root, POLICY_KEY, exe))
        .then_some(super::FolderOverride::Policy)
    }

    /// True when a `WebView2` `AdditionalBrowserArguments` policy for this app (machine or current user; the value
    /// named after the exe, else `*`, which applies to every app for this option) carries a proxy switch
    /// (`--proxy-server`, `--no-proxy-server`, `--proxy-bypass-list`, `--proxy-pac-url`, ...: any text with `proxy`,
    /// case ignored). `WebView2` appends the policy's arguments to the app's and Chromium uses the last copy of a
    /// switch, so such a policy takes previews off the network guard's proxy. Policy keys are written by
    /// admins only (a standard user can read them); the `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` variable, which the
    /// user can set, is removed at startup instead (`webview2_env`). Unreadable keys count as absent.
    #[must_use]
    pub fn policy_arguments_change_proxy() -> bool {
        let Some(exe) = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_owned))
        else {
            return false;
        };
        [
            windows_registry::LOCAL_MACHINE,
            windows_registry::CURRENT_USER,
        ]
        .into_iter()
        .any(|root| arguments_change_proxy(root, ARGUMENTS_KEY, &exe))
    }

    /// The registry key of the `AdditionalBrowserArguments` policy (one value per app: name = app id, exe name or `*`).
    const ARGUMENTS_KEY: &str =
        "Software\\Policies\\Microsoft\\Edge\\WebView2\\AdditionalBrowserArguments";

    /// True when `path` under `root` holds an arguments value for `exe` (or, without one, for `*`) that mentions a
    /// proxy; a value that is not text counts as one (failing towards refusing previews).
    fn arguments_change_proxy(root: &windows_registry::Key, path: &str, exe: &str) -> bool {
        let Ok(key) = root.open(path) else {
            return false;
        };
        let name = if key.get_type(exe).is_ok() { exe } else { "*" };
        if key.get_type(name).is_err() {
            return false;
        }
        key.get_string(name)
            .map_or(true, |text| text.to_ascii_lowercase().contains("proxy"))
    }

    /// The arguments-policy lookup of [`policy_arguments_change_proxy`] on a key of the current user's registry of the
    /// caller's choosing (`path`), for the tests (which must never write a real policy).
    #[doc(hidden)]
    #[must_use]
    pub fn arguments_change_proxy_at(path: &str, exe: &str) -> bool {
        arguments_change_proxy(windows_registry::CURRENT_USER, path, exe)
    }

    /// The registry key of the `UserDataFolder` policy (one value per app: name = app id or exe name).
    const POLICY_KEY: &str = "Software\\Policies\\Microsoft\\Edge\\WebView2\\UserDataFolder";

    /// True when `path` under `root` has a value named `exe` (any type: an empty or odd value still means a policy
    /// for this app, so it counts; failing towards "forced" refuses previews, never opens one unguarded).
    fn policy_names_folder(root: &windows_registry::Key, path: &str, exe: &str) -> bool {
        root.open(path).and_then(|key| key.get_type(exe)).is_ok()
    }

    /// The policy lookup of [`user_data_folder_override`] on a key of the current user's registry of the caller's
    /// choosing (`path`), for the tests (which must never write a real policy).
    #[doc(hidden)]
    #[must_use]
    pub fn policy_names_folder_at(path: &str, exe: &str) -> bool {
        policy_names_folder(windows_registry::CURRENT_USER, path, exe)
    }

    /// True when a setting replaces every `WebView2` environment's user data folder with one folder
    /// ([`user_data_folder_override`]): the environments then share one browser process, whose options must match.
    #[must_use]
    pub fn shared_user_data_folder() -> bool {
        user_data_folder_override().is_some()
    }

    /// Reads the storage `WebView2` gave the webview of `controller` (call on the UI thread, inside Tauri's
    /// `with_webview`).
    ///
    /// # Errors
    /// A text reason when the runtime is too old to report it (`ICoreWebView2_13` / `ICoreWebView2Environment7`,
    /// both in every Evergreen runtime since 2022) or a call fails: the caller then treats the storage as unknown.
    pub fn storage_facts(
        controller: &ICoreWebView2Controller,
        environment: &ICoreWebView2Environment,
    ) -> Result<StorageFacts, String> {
        let fail = |what: &str, e: windows::core::Error| format!("WebView2 {what}: {e}");
        // SAFETY: the controller and environment are Tauri's live WebView2 objects, handed over on the UI thread
        // inside `with_webview`. Each getter writes one CoTaskMem string that `take_pwstr` frees; every HRESULT
        // is checked.
        unsafe {
            let env7: ICoreWebView2Environment7 =
                environment.cast().map_err(|e| fail("environment (7)", e))?;
            let mut folder = PWSTR::null();
            env7.UserDataFolder(&raw mut folder)
                .map_err(|e| fail("user data folder", e))?;
            let user_data_folder = take_pwstr(folder);
            let core = controller.CoreWebView2().map_err(|e| fail("webview", e))?;
            let core13: ICoreWebView2_13 = core.cast().map_err(|e| fail("webview (13)", e))?;
            let profile = core13.Profile().map_err(|e| fail("profile", e))?;
            let mut name = PWSTR::null();
            profile
                .ProfileName(&raw mut name)
                .map_err(|e| fail("profile name", e))?;
            let profile_name = take_pwstr(name);
            let mut path = PWSTR::null();
            profile
                .ProfilePath(&raw mut path)
                .map_err(|e| fail("profile path", e))?;
            let profile_path = take_pwstr(path);
            Ok(StorageFacts {
                user_data_folder,
                profile_name,
                profile_path,
            })
        }
    }

    /// `WebView2` objects of one held request, moved to a helper thread and back. Only ever dereferenced on the UI
    /// thread: built in the handler there, used again only inside a closure [`PostToUi`] runs there.
    struct Held {
        args: ICoreWebView2WebResourceRequestedEventArgs,
        deferral: ICoreWebView2Deferral,
        env: ICoreWebView2Environment,
    }
    // SAFETY: see `Held`: the COM pointers cross threads only as opaque values and are touched on the UI thread.
    unsafe impl Send for Held {}

    /// Answers `args` with `403` (on the UI thread).
    ///
    /// # Safety
    /// Call on the UI thread with live event args and the webview's environment.
    unsafe fn refuse(
        env: &ICoreWebView2Environment,
        args: &ICoreWebView2WebResourceRequestedEventArgs,
    ) -> windows::core::Result<()> {
        // SAFETY: the caller's contract (UI thread, live objects).
        unsafe {
            let refused = env.CreateWebResourceResponse(
                None,
                403,
                &HSTRING::from("Forbidden"),
                &HSTRING::new(),
            )?;
            args.SetResponse(&refused)
        }
    }

    /// Answers every request the webview sends with `403` before it is sent when the rule refuses it: document
    /// requests (main frame and frames) and every other request (scripts, images, `fetch`, media, ...) ask `decide`
    /// (call on the UI thread, once per webview, before its first navigation). A [`Verdict::Later`] holds the request
    /// with a deferral and completes it on the UI thread through `post` when the verdict arrives, so a slow name
    /// lookup never freezes the window.
    ///
    /// # Errors
    /// A text reason when the filter or the handler cannot be registered: the caller must not show the webview.
    pub fn guard_requests(
        controller: &ICoreWebView2Controller,
        environment: &ICoreWebView2Environment,
        decide: DecideRequest,
        post: PostToUi,
    ) -> Result<(), String> {
        let fail =
            |what: &str, e: windows::core::Error| format!("WebView2 request guard ({what}): {e}");
        let env = environment.clone();
        let handler = WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else {
                return Ok(());
            };
            // SAFETY: WebView2 hands over live event args for the duration of this call on the UI thread; the
            // URI string is freed by `take_pwstr`; the response is created by the webview's own environment; a
            // deferral keeps the args alive until it is completed (on the UI thread, through `post`).
            unsafe {
                // Every handler sees every request that matches ANY filter of the webview (wry's custom-protocol
                // filters too: `ipc.localhost`, `tauri.localhost`); the rules refuse those for a preview page (it has
                // no IPC rights anyway).
                let mut context = COREWEBVIEW2_WEB_RESOURCE_CONTEXT::default();
                args.ResourceContext(&raw mut context)?;
                let mut uri = PWSTR::null();
                args.Request()?.Uri(&raw mut uri)?;
                let uri = take_pwstr(uri);
                match decide(&uri, context == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT) {
                    Verdict::Allow => {}
                    Verdict::Deny => refuse(&env, &args)?,
                    Verdict::Later(register) => {
                        let held = Held {
                            deferral: args.GetDeferral()?,
                            args: args.clone(),
                            env: env.clone(),
                        };
                        let post = post.clone();
                        register(Box::new(move |allowed| {
                            // This closure runs on the UI thread (`post`), where `held` was made (the SAFETY note
                            // of the enclosing block covers its calls).
                            post(Box::new(move || {
                                let held = held;
                                if !allowed {
                                    let _ = refuse(&held.env, &held.args);
                                }
                                let _ = held.deferral.Complete();
                            }));
                        }));
                    }
                }
            }
            Ok(())
        }));
        // SAFETY: the controller is Tauri's live WebView2 controller on the UI thread; the filter and the
        // handler registration only take owned or borrowed COM objects; every HRESULT is checked.
        unsafe {
            let core = controller.CoreWebView2().map_err(|e| fail("webview", e))?;
            let filter = HSTRING::from("*");
            // Since runtime 1.0.2592 frames need the request-source-kinds variant to be seen at all.
            match core.cast::<ICoreWebView2_22>() {
                Ok(core22) => core22
                    .AddWebResourceRequestedFilterWithRequestSourceKinds(
                        &filter,
                        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
                        COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
                    )
                    .map_err(|e| fail("filter", e))?,
                Err(_) => core
                    .AddWebResourceRequestedFilter(&filter, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)
                    .map_err(|e| fail("filter", e))?,
            }
            let mut token = Default::default();
            core.add_WebResourceRequested(&handler, &raw mut token)
                .map_err(|e| fail("handler", e))?;
        }
        Ok(())
    }

    /// Answers `done` once.
    fn answer(done: &Arc<Mutex<Option<PngDone>>>, result: Result<Vec<u8>, String>) {
        if let Some(done) = done.lock().unwrap_or_else(PoisonError::into_inner).take() {
            done(result);
        }
    }

    /// Decodes the `DevTools` answer `{"data": "<base64 png>"}`.
    fn decode(json: &str) -> Result<Vec<u8>, String> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| format!("DevTools answer is not JSON: {e}"))?;
        let data = value
            .get("data")
            .and_then(serde_json::Value::as_str)
            .ok_or("DevTools answer has no image data")?;
        base64::engine::general_purpose::STANDARD
            .decode(data.trim())
            .map_err(|e| format!("DevTools image is not base64: {e}"))
    }

    /// The page zoom the controller's webview has now (1 when it cannot say). Read, not remembered: `WebView2`
    /// brings a zoom outside its internal range into it, and the controller is what knows the zoom in force (the
    /// property keeps the asked value only until `ZoomFactorChanged`, far sooner than any capture).
    fn page_zoom(controller: &ICoreWebView2Controller) -> f64 {
        let mut zoom = 1.0;
        // SAFETY: `controller` is Tauri's live WebView2 controller on the UI thread; the getter only writes the
        // local out-parameter and its HRESULT is checked.
        match unsafe { controller.ZoomFactor(&raw mut zoom) } {
            Ok(()) => zoom,
            Err(_) => 1.0,
        }
    }

    /// Takes a PNG of the page shown by `controller`'s webview (call on the UI thread). Answers `done` later, on
    /// the UI thread, when `WebView2` has the image. `full`: the whole document of that CSS size in one render
    /// (`DevTools` `captureBeyondViewport` with a clip, as in ShellX), drawn one device pixel per CSS pixel whatever
    /// zoom the preview is shown at (`full_clip`); otherwise captures what the view shows.
    pub fn capture_png(
        controller: &ICoreWebView2Controller,
        full: Option<(u32, u32)>,
        done: PngDone,
    ) {
        let params = match full {
            Some(css) => HSTRING::from(full_clip(css, page_zoom(controller)).params()),
            None => HSTRING::from(r#"{"format":"png","fromSurface":true}"#),
        };
        let done = Arc::new(Mutex::new(Some(done)));
        let for_handler = done.clone();
        let handler =
            CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |result, json| {
                answer(
                    &for_handler,
                    result
                        .map_err(|e| format!("WebView2 screenshot failed: {e}"))
                        .and_then(|()| decode(&json)),
                );
                Ok(())
            }));
        // SAFETY: `controller` is Tauri's live WebView2 controller, handed to us inside `with_webview` on the UI
        // thread; both COM calls only read it, every HRESULT is checked, and the handler owns its state.
        let started = unsafe {
            controller.CoreWebView2().and_then(|core| {
                core.CallDevToolsProtocolMethod(w!("Page.captureScreenshot"), &params, &handler)
            })
        };
        if let Err(error) = started {
            answer(
                &done,
                Err(format!("WebView2 screenshot could not start: {error}")),
            );
        }
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos_impl {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};

    use block2::RcBlock;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
    use objc2_foundation::{NSDictionary, NSError};
    use objc2_web_kit::WKWebView;

    use super::{CookieIdentity, PngDone};

    /// Reads only names and domains from the `WKWebsiteDataStore` of Tauri's live view.
    pub fn provider_accounts(
        webview: *mut c_void,
        done: Box<dyn FnOnce(Result<Vec<CookieIdentity>, String>) + Send>,
    ) {
        if webview.is_null() {
            done(Err("the account browser is unavailable".to_owned()));
            return;
        }
        let done = Mutex::new(Some(done));
        let block = RcBlock::new(
            move |cookies: std::ptr::NonNull<
                objc2_foundation::NSArray<objc2_foundation::NSHTTPCookie>,
            >| {
                // SAFETY: WebKit supplies a valid NSArray for this callback; only metadata accessors are used.
                let identities = unsafe { cookies.as_ref() }
                    .iter()
                    .map(|cookie| CookieIdentity {
                        name: cookie.name().to_string(),
                        domain: cookie.domain().to_string(),
                    })
                    .collect();
                if let Some(done) = done.lock().unwrap_or_else(PoisonError::into_inner).take() {
                    done(Ok(identities));
                }
            },
        );
        // SAFETY: Tauri hands over a live WKWebView on the UI thread; WebKit retains the completion block.
        unsafe {
            let view = &*webview.cast::<WKWebView>();
            view.configuration()
                .websiteDataStore()
                .httpCookieStore()
                .getAllCookies(&block);
        }
    }

    /// Deletes every cookie in the fixed provider domains from the view's `WKWebsiteDataStore`.
    pub fn sign_out_provider(
        webview: *mut c_void,
        domains: Vec<String>,
        done: Box<dyn FnOnce(Result<(), String>) + Send>,
    ) {
        if webview.is_null() {
            done(Err("the account browser is unavailable".to_owned()));
            return;
        }
        // SAFETY: Tauri hands over a live WKWebView on the UI thread. The store is retained across asynchronous
        // callbacks by the captured Objective-C object; each delete completion decrements the count once.
        unsafe {
            let view = &*webview.cast::<WKWebView>();
            let store = view.configuration().websiteDataStore().httpCookieStore();
            let delete_store = store.clone();
            let done = Arc::new(Mutex::new(Some(done)));
            let block = RcBlock::new(
                move |cookies: std::ptr::NonNull<
                    objc2_foundation::NSArray<objc2_foundation::NSHTTPCookie>,
                >| {
                    // WebKit supplies a valid NSArray for the duration of this callback.
                    let cookies = cookies.as_ref();
                    let selected: Vec<_> = cookies
                        .iter()
                        .filter(|cookie| {
                            let domain = cookie.domain().to_string();
                            let domain = domain.trim_start_matches('.');
                            domains.iter().any(|allowed| {
                                domain.eq_ignore_ascii_case(allowed)
                                    || domain
                                        .to_ascii_lowercase()
                                        .ends_with(&format!(".{allowed}"))
                            })
                        })
                        .collect();
                    if selected.is_empty() {
                        if let Some(done) =
                            done.lock().unwrap_or_else(PoisonError::into_inner).take()
                        {
                            done(Ok(()));
                        }
                        return;
                    }
                    let left = Arc::new(AtomicUsize::new(selected.len()));
                    for cookie in selected {
                        let done = done.clone();
                        let left = left.clone();
                        let completion = RcBlock::new(move || {
                            if left.fetch_sub(1, Ordering::AcqRel) == 1
                                && let Some(done) =
                                    done.lock().unwrap_or_else(PoisonError::into_inner).take()
                            {
                                done(Ok(()));
                            }
                        });
                        // `cookie` is in the callback's live NSArray; WebKit retains it for deletion.
                        delete_store.deleteCookie_completionHandler(&cookie, Some(&completion));
                    }
                },
            );
            store.getAllCookies(&block);
        }
    }

    pub(super) fn png_of(image: &NSImage) -> Result<Vec<u8>, String> {
        let tiff = image.TIFFRepresentation().ok_or("snapshot has no bitmap")?;
        let rep = NSBitmapImageRep::imageRepWithData(&tiff).ok_or("snapshot bitmap unreadable")?;
        // SAFETY: an empty properties dictionary has the key and value types the method expects.
        let png = unsafe {
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        }
        .ok_or("snapshot could not be encoded as PNG")?;
        Ok(png.to_vec())
    }

    /// Takes a PNG of `webview` (Tauri's `PlatformWebview::inner()`, a `WKWebView`, on the main thread). Answers
    /// `done` when `WebKit` hands over the image.
    pub fn snapshot_png(webview: *mut c_void, done: PngDone) {
        if webview.is_null() {
            done(Err("the preview webview is not available".to_owned()));
            return;
        }
        let done = Mutex::new(Some(done));
        let block = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
            let Some(done) = done.lock().unwrap_or_else(PoisonError::into_inner).take() else {
                return;
            };
            // SAFETY: WebKit passes either a valid image or a valid error for the duration of this call.
            let result = match unsafe { image.as_ref() } {
                Some(image) => png_of(image),
                None => Err(unsafe { error.as_ref() }.map_or_else(
                    || "snapshot failed".to_owned(),
                    |e| e.localizedDescription().to_string(),
                )),
            };
            done(result);
        });
        // SAFETY: Tauri hands us its live `WKWebView` pointer on the main thread inside `with_webview`; it stays
        // alive for this call, and WebKit retains the block until it calls it.
        unsafe {
            let view = &*webview.cast::<WKWebView>();
            view.takeSnapshotWithConfiguration_completionHandler(None, &block);
        }
    }
}
