//! `WebKitGTK` script-dialog and HTTP Basic continuations, held by native references on the GTK thread.
//! Uses the shared dialog hub; proxy/TLS/other authentication stays with the existing native handlers.
//! Depends on: GTK references and the shared dialog hub.
//! Called by: the Linux webview dialog adapter.
use super::{Admit, Hub, hold};
use glib::translate::{ToGlibPtr, ToGlibPtrMut};
use serde_json::json;
use std::sync::Arc;
use webkit2gtk::{AuthenticationRequestExt, AuthenticationScheme, ScriptDialogType, WebViewExt};

/// Installs native dialog interception on one view before its first navigation.
///
/// # Errors
/// Reserved for native installation failures.
pub fn install(
    view: &webkit2gtk::WebView,
    label: String,
    hub: Arc<Hub>,
    admit: Admit,
) -> Result<(), String> {
    let (script_label, script_hub, script_admit) = (label.clone(), hub.clone(), admit.clone());
    view.connect_script_dialog(move |view, dialog| {
        if !script_admit() {
            return false;
        }
        let kind = match dialog.dialog_type() {
            ScriptDialogType::Alert => "alert",
            ScriptDialogType::Confirm => "confirm",
            ScriptDialogType::Prompt => "prompt",
            ScriptDialogType::BeforeUnloadConfirm => "beforeunload",
            _ => return false,
        };
        let default = (kind == "prompt")
            .then(|| dialog.prompt_get_default_text())
            .flatten()
            .map(|s| s.to_string());
        let info = json!({
            "type": kind,
            "message": dialog.message().as_deref().unwrap_or_default(),
            "defaultPrompt": default,
            "url": view.uri().as_deref().unwrap_or_default()
        });
        let dialog = dialog.clone();
        hold(
            &script_hub,
            &script_label,
            info,
            Box::new(move |answer| {
                match kind {
                    "confirm" | "beforeunload" => dialog.confirm_set_confirmed(answer.accept),
                    "prompt" if answer.accept => {
                        dialog.prompt_set_text(answer.text.as_deref().unwrap_or_default());
                    }
                    _ => {}
                }
                dialog.close();
                Ok(())
            }),
        )
    });
    view.connect_authenticate(move |view, request| {
        if request.is_for_proxy() || request.scheme() != AuthenticationScheme::HttpBasic || !admit()
        {
            return false;
        }
        let Some(origin) = request.security_origin() else {
            request.cancel();
            return true;
        };
        let info = json!({
            "type": "auth", "scheme": "basic", "message": "HTTP sign-in required",
            "realm": request.realm().as_deref().unwrap_or_default(),
            "origin": origin.to_string(),
            "url": view.uri().as_deref().unwrap_or_default(),
            "retry": request.is_retry()
        });
        request.set_can_save_credentials(false);
        let request = request.clone();
        hold(
            &hub,
            &label,
            info,
            Box::new(move |answer| {
                if let Some((username, password)) = answer.credentials.filter(|_| answer.accept) {
                    let mut credential = webkit2gtk::Credential::new(
                        &username,
                        &password,
                        webkit2gtk::CredentialPersistence::None,
                    );
                    // SAFETY: GTK owns both live native objects on this thread; authenticate copies the credential.
                    unsafe {
                        webkit2gtk::ffi::webkit_authentication_request_authenticate(
                            request.to_glib_none().0,
                            credential.to_glib_none_mut().0,
                        );
                    }
                } else {
                    request.cancel();
                }
                Ok(())
            }),
        )
    });
    Ok(())
}
