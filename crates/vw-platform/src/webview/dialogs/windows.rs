//! `WebView2` native script-dialog and HTTP Basic deferrals, kept on its UI thread.
//! Uses the shared dialog hub; existing proxy authentication is left untouched.
use super::{Admit, Hub, hold};
use serde_json::json;
use std::sync::Arc;
use webview2_com::{
    BasicAuthenticationRequestedEventHandler,
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_SCRIPT_DIALOG_KIND_ALERT, COREWEBVIEW2_SCRIPT_DIALOG_KIND_BEFOREUNLOAD,
        COREWEBVIEW2_SCRIPT_DIALOG_KIND_CONFIRM, COREWEBVIEW2_SCRIPT_DIALOG_KIND_PROMPT,
        ICoreWebView2, ICoreWebView2_10, ICoreWebView2Controller,
    },
    ScriptDialogOpeningEventHandler, take_pwstr,
};
use windows::core::{HSTRING, Interface, PWSTR};

/// Installs held dialogs on one native controller before navigation.
///
/// # Errors
/// The runtime has no required event interface or registration fails.
pub fn install(
    controller: &ICoreWebView2Controller,
    label: String,
    hub: Arc<Hub>,
    admit: Admit,
) -> Result<(), String> {
    // SAFETY: the live controller and all callbacks belong to WebView2's UI thread.
    unsafe {
        let core = controller.CoreWebView2().map_err(|e| e.to_string())?;
        core.Settings()
            .and_then(|s| s.SetAreDefaultScriptDialogsEnabled(!admit()))
            .map_err(|e| e.to_string())?;
        script(&core, label.clone(), hub.clone())?;
        authentication(&core, label, hub, admit)
    }
}

fn script(core: &ICoreWebView2, label: String, hub: Arc<Hub>) -> Result<(), String> {
    // SAFETY: registration and native args are used only on the UI thread; deferral is completed exactly once.
    unsafe {
        let mut token = 0;
        core.add_ScriptDialogOpening(
            &ScriptDialogOpeningEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else {
                    return Ok(());
                };
                let mut kind = COREWEBVIEW2_SCRIPT_DIALOG_KIND_ALERT;
                args.Kind(&raw mut kind)?;
                let kind = match kind {
                    COREWEBVIEW2_SCRIPT_DIALOG_KIND_ALERT => "alert",
                    COREWEBVIEW2_SCRIPT_DIALOG_KIND_CONFIRM => "confirm",
                    COREWEBVIEW2_SCRIPT_DIALOG_KIND_PROMPT => "prompt",
                    COREWEBVIEW2_SCRIPT_DIALOG_KIND_BEFOREUNLOAD => "beforeunload",
                    _ => return Ok(()),
                };
                let (mut message, mut url, mut default) =
                    (PWSTR::null(), PWSTR::null(), PWSTR::null());
                args.Message(&raw mut message)?;
                args.Uri(&raw mut url)?;
                args.DefaultText(&raw mut default)?;
                let info = json!({
                    "type": kind, "message": take_pwstr(message),
                    "url": take_pwstr(url), "defaultPrompt": take_pwstr(default)
                });
                let deferred = args.GetDeferral()?;
                hold(
                    &hub,
                    &label,
                    info,
                    Box::new(move |answer| {
                        let result = (|| -> windows::core::Result<()> {
                            if kind == "prompt" && answer.accept {
                                args.SetResultText(&HSTRING::from(
                                    answer.text.as_deref().unwrap_or_default(),
                                ))?;
                            }
                            if answer.accept || kind == "alert" {
                                args.Accept()?;
                            }
                            Ok(())
                        })();
                        let completed = deferred.Complete();
                        result
                            .and(completed)
                            .map_err(|_| "The native dialog could not confirm its answer.".into())
                    }),
                );
                Ok(())
            })),
            &raw mut token,
        )
        .map_err(|e| e.to_string())
    }
}

fn authentication(
    core: &ICoreWebView2,
    label: String,
    hub: Arc<Hub>,
    admit: Admit,
) -> Result<(), String> {
    // SAFETY: live UI-thread COM objects; credentials never enter metadata or diagnostics.
    unsafe {
        let core = core.cast::<ICoreWebView2_10>().map_err(|e| e.to_string())?;
        let mut token = 0;
        core.add_BasicAuthenticationRequested(
            &BasicAuthenticationRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else {
                    return Ok(());
                };
                if !admit() {
                    return Ok(());
                }
                let response = args.Response()?;
                let mut supplied = PWSTR::null();
                response.UserName(&raw mut supplied)?;
                if !take_pwstr(supplied).is_empty() {
                    return Ok(());
                }
                let (mut uri, mut challenge) = (PWSTR::null(), PWSTR::null());
                args.Uri(&raw mut uri)?;
                args.Challenge(&raw mut challenge)?;
                let (uri, challenge) = (take_pwstr(uri), take_pwstr(challenge));
                if !challenge.to_ascii_lowercase().starts_with("basic ") {
                    return Ok(());
                }
                let origin = url::Url::parse(&uri)
                    .map(|u| u.origin().ascii_serialization())
                    .unwrap_or_default();
                let deferred = args.GetDeferral()?;
                let info = json!({
                    "type": "auth", "scheme": "basic", "message": "HTTP sign-in required",
                    "realm": challenge, "origin": origin, "url": uri
                });
                hold(
                    &hub,
                    &label,
                    info,
                    Box::new(move |answer| {
                        let result = (|| -> windows::core::Result<()> {
                            if let Some((username, password)) =
                                answer.credentials.filter(|_| answer.accept)
                            {
                                response.SetUserName(&HSTRING::from(username.as_str()))?;
                                response.SetPassword(&HSTRING::from(password.as_str()))?;
                            } else {
                                args.SetCancel(true)?;
                            }
                            Ok(())
                        })();
                        let completed = deferred.Complete();
                        result
                            .and(completed)
                            .map_err(|_| "HTTP authentication was not completed.".into())
                    }),
                );
                Ok(())
            })),
            &raw mut token,
        )
        .map_err(|e| e.to_string())
    }
}

/// Sets script dialogs back to `WebView2`'s own UI when the person takes a page.
///
/// # Errors
/// The native runtime cannot update its settings.
pub fn ownership(controller: &ICoreWebView2Controller, owned: bool) -> Result<(), String> {
    // SAFETY: called only on the live controller's UI thread.
    unsafe {
        controller
            .CoreWebView2()
            .and_then(|core| core.Settings())
            .and_then(|settings| settings.SetAreDefaultScriptDialogsEnabled(!owned))
            .map_err(|_| "Could not update native page dialog ownership.".into())
    }
}
