//! The native dialogs the app owns: the confirmation for calls that need one (Rust shows it;
//! the webview and the Debug API can only ask, never answer), the folder picker, the document
//! picker, the library file picker, the certificate file picker of Settings > Network and the save dialog of
//! "Share as one file".
//!
//! Depends on: tauri, tauri-plugin-dialog, vw-host (`SharedConfirmer`), vw-gate (request type).
//! Called by: `lib.rs` (builds the confirmer for the host), `commands.rs` (`pick_folder`,
//! `pick_documents`, `pick_certificate`).
//! Invariants:
//! - Both dialogs are parented to the main window when it exists, so they are modal over it.
//! - A closed dialog, a failed dialog and a dropped channel all count as "declined" / "no folder":
//!   the confirmer fails closed.
//! - The text of a confirmation is built from the gate's request (`vw_host::dialog_text`): callers supply complete
//!   plain explanations and command paragraphs; internal methods and JSON are never displayed. A request too long
//!   for that (whoever asks) is shown visibly shortened and paged through in full before Allow.

use std::path::PathBuf;

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_dialog::DialogExt;
#[cfg(not(target_os = "linux"))]
use tauri_plugin_dialog::{MessageDialogButtons, MessageDialogKind, MessageDialogResult};
use tokio::sync::oneshot;
use vw_gate::{Confirmation, ConfirmationRequest};
use vw_host::SharedConfirmer;

#[cfg(target_os = "linux")]
mod linux_confirmation;

/// Shows one confirmation dialog with `text` and the buttons `labels` (first = default) and returns the pressed label;
/// `None` when it was closed, failed, or there are not 1 to 3 labels.
async fn show_dialog<R: Runtime>(
    app: &AppHandle<R>,
    text: String,
    labels: &[String],
) -> Option<String> {
    #[cfg(target_os = "linux")]
    return linux_confirmation::show_dialog(app, text, labels).await;

    #[cfg(not(target_os = "linux"))]
    {
        let (send, receive) = oneshot::channel();
        let buttons = match labels {
            // In `dialog_buttons` order: the first is the default (Return) on every OS. For an MCP
            // server's key that is "always", elsewhere "Allow once".
            [first, second, dont] => {
                MessageDialogButtons::YesNoCancelCustom(first.clone(), second.clone(), dont.clone())
            }
            [ok, cancel] => MessageDialogButtons::OkCancelCustom(ok.clone(), cancel.clone()),
            // A request too long to page through has one button (Close, which declines).
            [only] => MessageDialogButtons::OkCustom(only.clone()),
            _ => return None,
        };
        let mut dialog = app
            .dialog()
            .message(text)
            .title("Vibe Work asks for your confirmation")
            .kind(MessageDialogKind::Warning)
            .buttons(buttons);
        if let Some(window) = app.get_webview_window("main") {
            // A background macOS sheet can otherwise wait unseen. Present the native question to the
            // user; native input itself never brings a window forward or uses this path to obtain focus.
            #[cfg(target_os = "macos")]
            {
                let _ = window.show();
                let _ = window.set_focus();
            }
            dialog = dialog.parent(&window);
        }
        dialog.show_with_result(move |result| {
            let _ = send.send(result);
        });
        match receive.await.ok() {
            Some(MessageDialogResult::Custom(label)) => Some(label),
            // Platforms that answer the standard results for the first button.
            Some(MessageDialogResult::Ok | MessageDialogResult::Yes) => labels.first().cloned(),
            _ => None,
        }
    }
}

/// A shortened request (`vw_host::shortened_dialog_text`): the overview (start, marker,
/// end; default "Show the full request"), then every part of the full text in turn ("Next part"), and Allow only on
/// the last part. Each step is one native dialog; anything but the forward button declines.
async fn shortened<R: Runtime>(app: &AppHandle<R>, request: &ConfirmationRequest) -> Confirmation {
    let mut step = vw_host::ShortStep::Overview;
    // At most the overview and every page: the step only moves forward.
    for _ in 0..=vw_host::MAX_PAGES + 1 {
        let Some(text) = vw_host::shortened_dialog_text(request, step) else {
            break;
        };
        let labels = vw_host::shortened_buttons(request, step);
        let pressed = show_dialog(app, text, &labels).await;
        match vw_host::shortened_answer(request, step, pressed.as_deref()) {
            vw_host::ShortOutcome::Ask(next) => step = next,
            vw_host::ShortOutcome::Done(answer) => return answer,
        }
    }
    Confirmation::Declined
}

/// The confirmer the host hands to the core: shows [`vw_host::dialog_text`] in a native warning
/// dialog with Continue / Cancel buttons (three labelled buttons when the request offers "always"; their
/// order and default: `vw_host::dialog_buttons`). A request too long to show in full is never clipped (the hidden
/// part could be the real command) and never declined unseen: it is shown shortened and can be allowed only after
/// the user paged through all of it ([`shortened`]), so a caller's "the user declined" is always true.
pub fn native_confirmer<R: Runtime>(app: AppHandle<R>) -> SharedConfirmer {
    SharedConfirmer::new(move |request: ConfirmationRequest| {
        let app = app.clone();
        async move {
            let Some(text) = vw_host::dialog_text(&request) else {
                if vw_host::full_pages(&request).is_some() {
                    return shortened(&app, &request).await;
                }
                // No caller text at all (an empty summary or an unknown capture field): nothing to show, so no consent.
                tracing::warn!(method = %request.method, "confirmation declined: the request has no text to show");
                return Confirmation::Declined;
            };
            // A request that offers "always" gets three labelled buttons (`vw_host::dialog_buttons`); the
            // pressed label decides (`vw_host::dialog_answer`, tested in the private host tests).
            let labels = vw_host::dialog_buttons(&request);
            let pressed = show_dialog(&app, text, &labels).await;
            vw_host::dialog_answer(&request, pressed.as_deref())
        }
    })
}

/// The file types the document picker offers; the core checks the bytes again.
const DOCUMENT_TYPES: &[&str] = &[
    "pdf", "docx", "pptx", "xlsx", "csv", "txt", "md", "png", "jpg", "jpeg", "webp", "gif", "tif",
    "tiff", "bmp", "avif",
];

/// Opens the native file picker for documents (several files allowed) and returns the chosen
/// files; empty when cancelled. The webview supplies nothing.
pub async fn pick_documents<R: Runtime>(app: &AppHandle<R>) -> Vec<PathBuf> {
    let (send, receive) = oneshot::channel();
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Choose documents to send with your message")
        .add_filter("Documents and images", DOCUMENT_TYPES);
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&window);
    }
    dialog.pick_files(move |chosen| {
        let paths = chosen
            .unwrap_or_default()
            .into_iter()
            .filter_map(|path| path.into_path().ok())
            .collect::<Vec<_>>();
        let _ = send.send(paths);
    });
    receive.await.unwrap_or_default()
}

/// The file types the certificate picker offers; the core reads the file again and checks that it holds a PEM
/// certificate before anything is saved.
const CERTIFICATE_TYPES: &[&str] = &["pem", "crt", "cer", "cert"];

/// Opens the native file picker for a company's certificate file (one file) and returns it; `None` when cancelled.
/// The path is shown to the user and checked by the core when the setting is saved; the webview names no file.
pub async fn pick_certificate<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    let (send, receive) = oneshot::channel();
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Choose your company's certificate file")
        .add_filter("Certificate files", CERTIFICATE_TYPES);
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&window);
    }
    dialog.pick_file(move |chosen| {
        let _ = send.send(chosen.and_then(|path| path.into_path().ok()));
    });
    receive.await.ok().flatten()
}

/// Opens the native file picker for a library upload (several files, any type Mistral's Libraries
/// take) and returns the chosen files; empty when cancelled. The webview supplies nothing.
pub async fn pick_library_files<R: Runtime>(app: &AppHandle<R>) -> Vec<PathBuf> {
    let (send, receive) = oneshot::channel();
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Choose files to add to the library (they are sent to Mistral)");
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&window);
    }
    dialog.pick_files(move |chosen| {
        let paths = chosen
            .unwrap_or_default()
            .into_iter()
            .filter_map(|path| path.into_path().ok())
            .collect::<Vec<_>>();
        let _ = send.send(paths);
    });
    receive.await.unwrap_or_default()
}

/// Opens the native folder picker and returns the chosen folder, or `None` when cancelled.
pub async fn pick_folder<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    let (send, receive) = oneshot::channel();
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Choose the folder Vibe should work in");
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&window);
    }
    dialog.pick_folder(move |chosen| {
        let _ = send.send(chosen.and_then(|path| path.into_path().ok()));
    });
    receive.await.ok().flatten()
}

/// Opens the native save dialog for a session exported as Markdown with `name` suggested; returns the
/// chosen file, or `None` when cancelled. The webview never supplies the destination.
pub async fn save_markdown<R: Runtime>(app: &AppHandle<R>, name: &str) -> Option<PathBuf> {
    let (send, receive) = oneshot::channel();
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Export the session as Markdown")
        .set_file_name(name)
        .add_filter("Markdown", &["md"]);
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&window);
    }
    dialog.save_file(move |chosen| {
        let _ = send.send(chosen.and_then(|path| path.into_path().ok()));
    });
    receive.await.ok().flatten()
}

/// Opens the native save dialog for an exported handbook page with `name` suggested and returns the chosen
/// file, or `None` when cancelled. The webview supplies only the suggested name's page, never the destination.
pub async fn save_page<R: Runtime>(app: &AppHandle<R>, name: &str) -> Option<PathBuf> {
    let (send, receive) = oneshot::channel();
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Save the page as one file")
        .set_file_name(name)
        .add_filter("Web page", &["html"]);
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&window);
    }
    dialog.save_file(move |chosen| {
        let _ = send.send(chosen.and_then(|path| path.into_path().ok()));
    });
    receive.await.ok().flatten()
}
