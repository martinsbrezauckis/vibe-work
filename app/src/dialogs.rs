//! The native dialogs the app owns: the confirmation for calls that need one (Rust shows it;
//! the webview and the Debug API can only ask, never answer), the folder picker, the document
//! picker, the library file picker, the certificate file picker of Settings > Network and the save dialog of
//! "Share as one file".
//!
//! Depends on: tauri, tauri-plugin-dialog (pickers), our own confirmation window per OS (GTK on Linux, an `AppKit`
//! alert sheet on macOS, a common-controls task dialog on Windows), vw-host (`SharedConfirmer`, button order, heading),
//! vw-gate (request type).
//! Called by: `lib.rs` (builds the confirmer for the host), `commands.rs` (`pick_folder`,
//! `pick_documents`, `pick_certificate`).
//! Invariants:
//! - A confirmation is modal over the main window when it exists, except an agent's request (Git guard, kill guard,
//!   vault, website login, tabs): that one is a window of its own, so only the waiting chat is held and the person
//!   can work in other chats meanwhile. Pickers are always modal over the main window.
//! - Such a window can go behind the main window; [`raise_prompt`] brings it back by its ask id (the window does so
//!   when its chat is shown, and from the chat's "waiting" hint). Raising never answers. Only a modal confirmation
//!   brings the main window forward when it opens ([`present_for`]); an agent's window of its own never does: the
//!   chat's badge and the app badge say it waits.
//! - A closed dialog, a failed dialog and a dropped channel all count as "declined" / "no folder":
//!   the confirmer fails closed. The declining button is the default (Return) on every OS, so a stray Enter never
//!   approves; Escape declines on every OS; a screen reader's or voice control's press is the button's own click.
//! - A request that ends without an answer (a cancelled call) closes its window on every OS.
//! - The text of a confirmation is built from the gate's request (`vw_host::dialog_text`): callers supply complete
//!   plain explanations and command paragraphs; internal methods and JSON are never displayed. A request too long
//!   for that (whoever asks) is shown visibly shortened and paged through in full before Allow.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_dialog::{DialogExt, FileDialogBuilder, FilePath};
use tokio::sync::oneshot;
use vw_gate::{Confirmation, ConfirmationRequest};
use vw_host::SharedConfirmer;

#[cfg(target_os = "linux")]
mod linux_confirmation;
#[cfg(target_os = "macos")]
mod macos_confirmation;
#[cfg(windows)]
mod windows_confirmation;

/// Brings one shown window to the front; false when it is gone.
type Raise = Arc<dyn Fn() -> bool + Send + Sync>;

/// The open confirmation windows of their own, by ask id (`params.askId`, which the core sets to the ask's id), in
/// the order they were shown. Only listed while their window exists.
static OWN_WINDOWS: Mutex<Vec<(u64, String, Raise)>> = Mutex::new(Vec::new());
static NEXT_LISTED: AtomicU64 = AtomicU64::new(1);

/// One entry of [`OWN_WINDOWS`]; dropping it takes the entry out.
pub(crate) struct Listed(u64);

impl Drop for Listed {
    fn drop(&mut self) {
        OWN_WINDOWS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(token, _, _)| *token != self.0);
    }
}

/// Lists a shown window of its own for ask `ask` with the way to bring it to the front (`None` when the request
/// names no ask: nothing to list).
pub(crate) fn listed(ask: Option<&str>, raise: Raise) -> Option<Listed> {
    let ask = ask?;
    let token = NEXT_LISTED.fetch_add(1, Ordering::Relaxed);
    OWN_WINDOWS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push((token, ask.to_owned(), raise));
    Some(Listed(token))
}

/// Brings the confirmation window of ask `ask` to the front; false when no such window is open.
pub fn raise_prompt(ask: &str) -> bool {
    let raise = OWN_WINDOWS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .find(|(_, id, _)| id == ask)
        .map(|(_, _, raise)| raise.clone());
    raise.is_some_and(|raise| raise())
}

/// The ask id a request carries for [`listed`].
fn ask_of(request: &ConfirmationRequest) -> Option<&str> {
    request
        .params
        .get("askId")
        .and_then(serde_json::Value::as_str)
}

/// Registers only app-modal native windows; independent chat approvals do not block other chats.
fn hold_modal<R: Runtime>(app: &AppHandle<R>, title: &str) -> Option<vw_core_api::BrowserDialog> {
    let host = app.try_state::<vw_host::HostHandle>()?.try_get()?;
    Some(host.core()?.browser_dialog(title))
}

/// Shows one confirmation dialog with `text` and the buttons `labels` (last = declining = default, see
/// `vw_host::dialog_buttons`) and returns the pressed label; `None` when it was closed, declined without a button
/// (Escape), failed, or there are not 1 to 3 labels. Dropping the future closes the window.
/// `ask`: the ask id a window of its own (`modal` false) is listed under for [`raise_prompt`].
async fn show_dialog<R: Runtime>(
    app: &AppHandle<R>,
    text: String,
    labels: &[String],
    modal: bool,
    ask: Option<&str>,
) -> Option<String> {
    let _dialog = modal.then(|| hold_modal(app, &vw_host::dialog_heading(&text).0));
    let ask = ask.filter(|_| !modal);
    #[cfg(target_os = "linux")]
    return linux_confirmation::show_dialog(app, text, labels, modal, ask).await;
    #[cfg(target_os = "macos")]
    return macos_confirmation::show_dialog(app, text, labels, modal, ask).await;
    #[cfg(windows)]
    return windows_confirmation::show_dialog(app, text, labels, modal, ask).await;
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
        let pressed = show_dialog(app, text, &labels, modal(request), ask_of(request)).await;
        match vw_host::shortened_answer(request, step, pressed.as_deref()) {
            vw_host::ShortOutcome::Ask(next) => step = next,
            vw_host::ShortOutcome::Done(answer) => return answer,
        }
    }
    Confirmation::Declined
}

/// `modal` for [`show_dialog`]: an agent's request holds only its own chat, so it is a window of its own.
fn modal(request: &ConfirmationRequest) -> bool {
    request.requester != vw_gate::Requester::Agent
}

/// Whether a confirmation brings the main window forward before it shows: only a modal one (a sheet or a dialog
/// over the main window, which could otherwise wait unseen on a hidden or background window). An agent's request
/// is a window of its own that never brings the app forward or takes the focus from another app: the waiting
/// chat's badge and the app badge (Dock, taskbar, tray) say that it waits, and showing that chat raises it.
#[cfg(any(test, not(target_os = "linux")))]
const fn reveals_main_window(modal: bool) -> bool {
    modal
}

/// Brings the main window forward for a confirmation when [`reveals_main_window`] says so; the per-OS windows call
/// this instead of revealing by themselves.
#[cfg(not(target_os = "linux"))]
fn present_for<R: Runtime>(app: &AppHandle<R>, modal: bool) {
    if reveals_main_window(modal) {
        let _ = crate::tray::reveal_main_window(app);
    }
}

/// The confirmer the host hands to the core: shows [`vw_host::dialog_text`] in a native warning
/// dialog with Continue / Cancel buttons (three labelled buttons when the request offers "always"; their
/// order: `vw_host::dialog_buttons`; the declining one is the default on every OS). A request too long to show in full is never clipped (the hidden
/// part could be the real command) and never declined unseen: it is shown shortened and can be allowed only after
/// the user paged through all of it ([`shortened`]), so a caller's "the user declined" is always true.
pub fn native_confirmer<R: Runtime>(app: AppHandle<R>) -> SharedConfirmer {
    SharedConfirmer::new(move |request: ConfirmationRequest| {
        let app = app.clone();
        async move {
            vw_host::foreground_confirmation(&request, || {
                if let Err(error) = crate::tray::reveal_main_window(&app) {
                    tracing::warn!(%error, "pairing confirmation could not bring the app forward");
                }
            });
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
            let pressed = show_dialog(&app, text, &labels, modal(&request), ask_of(&request)).await;
            vw_host::dialog_answer(&request, pressed.as_deref())
        }
    })
}

/// The file types the document picker offers; the core checks the bytes again.
const DOCUMENT_TYPES: &[&str] = &[
    "pdf", "docx", "pptx", "xlsx", "csv", "txt", "md", "png", "jpg", "jpeg", "webp", "gif", "tif",
    "tiff", "bmp", "avif",
];

/// What the attach dialog shows: one title and one filter name on every OS, whichever route opens it (the paperclip
/// and the document picker both use this; a dialog the webview draws itself cannot carry either).
pub(crate) struct PickerSpec {
    pub title: &'static str,
    pub filter: &'static str,
    pub types: &'static [&'static str],
}

/// The document picker's dialog.
pub(crate) const DOCUMENTS_PICKER: PickerSpec = PickerSpec {
    title: "Choose documents to send with your message",
    filter: "Documents and images",
    types: DOCUMENT_TYPES,
};

/// Which one-path dialog [`choose_one`] shows.
enum Chooser {
    File,
    Folder,
    Save,
}

/// A file dialog titled `title` over the main window, with `configure`'s filters and names. Every picker and save
/// dialog of the app goes through [`choose_one`] or [`choose_files`]: held as the app's open dialog (`hold_modal`)
/// while it shows, the chosen path(s) back, none when cancelled.
fn file_dialog<R: Runtime>(
    app: &AppHandle<R>,
    title: &str,
    configure: impl FnOnce(FileDialogBuilder<R>) -> FileDialogBuilder<R>,
) -> FileDialogBuilder<R> {
    let mut dialog = configure(app.dialog().file().set_title(title));
    if let Some(window) = crate::main_window(app) {
        dialog = dialog.set_parent(&window);
    }
    dialog
}

/// Shows a one-path dialog (`kind`) and returns the chosen path; `None` when cancelled.
async fn choose_one<R: Runtime>(
    app: &AppHandle<R>,
    title: &str,
    kind: Chooser,
    configure: impl FnOnce(FileDialogBuilder<R>) -> FileDialogBuilder<R>,
) -> Option<PathBuf> {
    let modal = hold_modal(app, title);
    let (send, receive) = oneshot::channel();
    let dialog = file_dialog(app, title, configure);
    let reply = move |chosen: Option<FilePath>| {
        drop(modal);
        let _ = send.send(chosen.and_then(|path| path.into_path().ok()));
    };
    match kind {
        Chooser::File => dialog.pick_file(reply),
        Chooser::Folder => dialog.pick_folder(reply),
        Chooser::Save => dialog.save_file(reply),
    }
    receive.await.ok().flatten()
}

/// Shows a several-files picker and returns the chosen files; empty when cancelled.
async fn choose_files<R: Runtime>(
    app: &AppHandle<R>,
    title: &str,
    configure: impl FnOnce(FileDialogBuilder<R>) -> FileDialogBuilder<R>,
) -> Vec<PathBuf> {
    let modal = hold_modal(app, title);
    let (send, receive) = oneshot::channel();
    file_dialog(app, title, configure).pick_files(move |chosen| {
        drop(modal);
        let paths = chosen
            .unwrap_or_default()
            .into_iter()
            .filter_map(|path| path.into_path().ok())
            .collect::<Vec<_>>();
        let _ = send.send(paths);
    });
    receive.await.unwrap_or_default()
}

/// Opens the native file picker for documents (several files allowed) and returns the chosen
/// files; empty when cancelled. The webview supplies nothing.
pub async fn pick_documents<R: Runtime>(app: &AppHandle<R>) -> Vec<PathBuf> {
    choose_files(app, DOCUMENTS_PICKER.title, |dialog| {
        dialog.add_filter(DOCUMENTS_PICKER.filter, DOCUMENTS_PICKER.types)
    })
    .await
}

/// The file types the certificate picker offers; the core reads the file again and checks that it holds a PEM
/// certificate before anything is saved.
const CERTIFICATE_TYPES: &[&str] = &["pem", "crt", "cer", "cert"];

/// Opens the native file picker for a company's certificate file (one file) and returns it; `None` when cancelled.
/// The path is shown to the user and checked by the core when the setting is saved; the webview names no file.
pub async fn pick_certificate<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    choose_one(
        app,
        "Choose your company's certificate file",
        Chooser::File,
        |dialog| dialog.add_filter("Certificate files", CERTIFICATE_TYPES),
    )
    .await
}

/// Opens the native file picker for a library upload (several files, any type Mistral's Libraries
/// take) and returns the chosen files; empty when cancelled. The webview supplies nothing.
pub async fn pick_library_files<R: Runtime>(app: &AppHandle<R>) -> Vec<PathBuf> {
    choose_files(
        app,
        "Choose files to add to the library (they are sent to Mistral)",
        |dialog| dialog,
    )
    .await
}

/// What a picked folder is for; the webview names one of these (`work`, `knowledge`), the titles stay in the host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FolderPurpose {
    /// A project folder: Vibe works in it.
    #[default]
    Work,
    /// A Knowledge base place: the agent only reads it.
    Knowledge,
}

impl FolderPurpose {
    /// The purpose the webview named; `None` for any other word.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "work" => Some(Self::Work),
            "knowledge" => Some(Self::Knowledge),
            _ => None,
        }
    }
}

/// The native folder picker's title for `purpose` (shared vocabulary).
fn folder_title(purpose: FolderPurpose) -> &'static str {
    match purpose {
        FolderPurpose::Work => vw_vocabulary::texts::FOLDER_PICKER_WORK,
        FolderPurpose::Knowledge => vw_vocabulary::texts::FOLDER_PICKER_KNOWLEDGE,
    }
}

/// Opens the native folder picker titled for `purpose` and returns the chosen folder, or `None` when cancelled.
pub async fn pick_folder<R: Runtime>(
    app: &AppHandle<R>,
    purpose: FolderPurpose,
) -> Option<PathBuf> {
    choose_one(app, folder_title(purpose), Chooser::Folder, |dialog| dialog).await
}

/// Opens the native save dialog for a session exported as Markdown with `name` suggested; returns the
/// chosen file, or `None` when cancelled. The webview never supplies the destination.
pub async fn save_markdown<R: Runtime>(app: &AppHandle<R>, name: &str) -> Option<PathBuf> {
    choose_one(
        app,
        "Export the session as Markdown",
        Chooser::Save,
        |dialog| dialog.set_file_name(name).add_filter("Markdown", &["md"]),
    )
    .await
}

/// Opens the native save dialog for an exported handbook page with `name` suggested and returns the chosen
/// file, or `None` when cancelled. The webview supplies only the suggested name's page, never the destination.
pub async fn save_page<R: Runtime>(app: &AppHandle<R>, name: &str) -> Option<PathBuf> {
    choose_one(app, "Save the page as one file", Chooser::Save, |dialog| {
        dialog.set_file_name(name).add_filter("Web page", &["html"])
    })
    .await
}

#[cfg(test)]
mod picker_spec_tests {
    //! The attachment picker offers the same title, filter and supported file types.
    use super::DOCUMENTS_PICKER;

    #[test]
    fn the_attach_dialog_names_its_title_and_filter_and_offers_images_and_documents() {
        assert_eq!(
            DOCUMENTS_PICKER.title,
            "Choose documents to send with your message"
        );
        assert_eq!(DOCUMENTS_PICKER.filter, "Documents and images");
        for kind in [
            "pdf", "docx", "pptx", "xlsx", "csv", "txt", "md", "png", "jpg",
        ] {
            assert!(DOCUMENTS_PICKER.types.contains(&kind), "{kind}");
        }
    }
}

#[cfg(test)]
#[path = "dialogs/present_tests.rs"]
mod present_tests;

#[cfg(test)]
mod folder_title_tests {
    //! PREDICT: did not compile before the change: one fixed title served every folder picker, so a Knowledge base
    //! place was asked for with the project-folder title.
    use super::{FolderPurpose, folder_title};

    #[test]
    fn each_folder_purpose_has_its_own_title_and_the_default_is_the_project_folder() {
        let purpose = |name: &str| FolderPurpose::named(name).unwrap();
        let text = |id: &str| vw_vocabulary::text(id).unwrap();
        assert_eq!(
            folder_title(purpose("knowledge")),
            text("folderPickerKnowledge")
        );
        assert_eq!(folder_title(purpose("work")), text("folderPickerWork"));
        assert_ne!(
            folder_title(FolderPurpose::Knowledge),
            folder_title(FolderPurpose::Work)
        );
        assert_eq!(
            folder_title(FolderPurpose::default()),
            folder_title(FolderPurpose::Work)
        );
        assert_eq!(FolderPurpose::named("Choose anything"), None);
    }
}
