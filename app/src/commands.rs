//! The Tauri commands the main window may call: the whole bridge between the webview and the
//! core. Each one only forwards to [`vw_host::Host`]; no behaviour lives here.
//!
//! Capability: `capabilities/main.json` grants exactly these commands to the `main` window
//! (`build.rs` lists them). Called by: the UI (`ui/src/protocol/client/tauriClient.ts`).
//! Invariants:
//! - `core_execute` runs every command as the window's (`origin = ui`). The webview cannot claim the Debug API
//!   origin (which acts as the user without native dialogs): no argument of these commands carries such a claim,
//!   and any extra argument a script adds is ignored. Only the host turns one window command into the Debug API's,
//!   when it matches a grant a pending Debug API `ui/*` request got in Rust from its own route and body
//!   (`vw_host::Host::execute_window`, `vw_host::ui_grants`). The update commands are always the window's.
//! - No command returns a secret, a token or the Debug API descriptor.

use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Manager, State};
use vw_core_api::{Command, CommandResult, CoreError, CoreEvent, ErrorCode};
use vw_host::update_flow::{UpdateManager, UpdateRequester, UpdateView};
use vw_host::{HostEvent, HostHandle, NotifyRequest, TerminalAttached, UiReply, UiRequest};

use crate::dialogs;
use crate::updater::UpdatesHandle;

/// Executes one core command for the window (`origin = ui`; the host's grants aside, module docs).
///
/// # Errors
/// The core's [`CoreError`] (serialized to the JS side as the rejection value).
#[tauri::command]
pub async fn core_execute(
    host: State<'_, HostHandle>,
    command: Command,
) -> Result<CommandResult, CoreError> {
    host.get_for_window().await.execute_window(command).await
}

/// Starts (or restarts) the event stream into `events`: first a snapshot of every loaded session,
/// then ordered changes and core events. A second call replaces the first stream.
///
/// # Errors
/// `serverUnavailable` when Vibe is not running (the UI then shows the reason).
#[tauri::command]
pub async fn core_subscribe(
    host: State<'_, HostHandle>,
    events: Channel<CoreEvent>,
) -> Result<(), CoreError> {
    host.get()
        .await
        .subscribe(move |event| events.send(event).is_ok())
}

/// Long-polls for the next Debug API `ui/*` request (up to `wait_ms`, capped at 30 s). `None`
/// means nothing arrived and the UI polls again.
#[tauri::command]
pub async fn ui_control_next(
    host: State<'_, HostHandle>,
    wait_ms: Option<u64>,
) -> Result<Option<UiRequest>, CoreError> {
    let host = host.get().await;
    let wait = std::time::Duration::from_millis(wait_ms.unwrap_or(25_000));
    Ok(host.ui().next_request(wait).await)
}

/// Answers a `ui/*` request. Returns whether the answer was accepted (`false` for an unknown or
/// finished id).
#[tauri::command]
pub async fn ui_control_reply(
    host: State<'_, HostHandle>,
    reply: UiReply,
) -> Result<bool, CoreError> {
    let (id, outcome) = reply.into_parts();
    Ok(host.get().await.ui().reply(&id, outcome))
}

/// Turns the Debug API on or off and remembers the choice (the Settings switch). No
/// dialog: the bearer token is the gate, and the webview cannot read it.
///
/// # Errors
/// `internal` when the setting cannot be saved.
#[tauri::command]
pub async fn debug_api_set(host: State<'_, HostHandle>, enabled: bool) -> Result<(), CoreError> {
    host.get()
        .await
        .set_debug_api(enabled)
        .await
        .map_err(|error| {
            CoreError::new(
                ErrorCode::Internal,
                format!("cannot save the setting: {error}"),
            )
        })
}

/// The CLI sharing switch (Settings): saves `share` first when it is `Some`, then returns the
/// switch, whether the CLI home exists and the CLI's version next to the bundled one. Takes effect the next
/// time Vibe Work starts.
///
/// # Errors
/// `internal` when the setting cannot be saved.
#[tauri::command]
pub async fn cli_sharing(
    app: AppHandle,
    host: State<'_, HostHandle>,
    share: Option<bool>,
) -> Result<vw_host::cli_sharing::CliSharingView, CoreError> {
    let host = host.get().await;
    if let Some(share) = share {
        host.set_cli_sharing(share).map_err(|error| {
            CoreError::new(
                ErrorCode::Internal,
                format!("cannot save the setting: {error}"),
            )
        })?;
    }
    let probe = vw_host::cli_sharing::CliProbe {
        cli_home: crate::startup::cli_home(&app),
        path_var: std::env::var_os("PATH"),
        home: app.path().home_dir().ok(),
    };
    Ok(host.cli_sharing(&probe).await)
}

/// The Debug API token file's PATH (shown in Settings), regenerating the token first when `rotate`
/// is true. Never returns the token itself.
///
/// # Errors
/// `internal` when a new token cannot be written user-only.
#[tauri::command]
pub async fn debug_api_token_file(
    host: State<'_, HostHandle>,
    rotate: bool,
) -> Result<String, CoreError> {
    host.get()
        .await
        .debug_token_file(rotate)
        .await
        .map(|path| path.display().to_string())
        .map_err(|error| CoreError::new(ErrorCode::Internal, error.to_string()))
}

/// Starts (or restarts) the stream of host events (an OS notification was clicked). A second call
/// replaces the first stream.
#[tauri::command]
pub async fn host_subscribe(
    app: AppHandle,
    host: State<'_, HostHandle>,
    events: Channel<HostEvent>,
) -> Result<(), CoreError> {
    host.get().await.subscribe_host(move |event| {
        // A clicked notification should bring the window forward; Windows and macOS already do
        // it, a Linux desktop often does not.
        if let Some(window) = crate::main_window(&app) {
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
        events.send(event).is_ok()
    });
    Ok(())
}

/// Opens the native folder picker (Rust owns it: the webview supplies no path), registers the
/// chosen folder with the gate as user-picked, and returns it. `None` when the user cancelled.
///
/// # Errors
/// `invalidParams` when the chosen path is not usable as a folder.
#[tauri::command]
pub async fn pick_folder(
    app: AppHandle,
    host: State<'_, HostHandle>,
) -> Result<Option<String>, CoreError> {
    let Some(path) = dialogs::pick_folder(&app).await else {
        return Ok(None);
    };
    host.get().await.register_picked_folder(&path).map(Some)
}

/// Opens the native file picker for a company's certificate file (Rust owns the dialog) and returns the chosen path,
/// or `None` when the user cancelled. The path is not a secret; the core checks the file when Settings > Network saves it.
///
/// # Errors
/// None today; the signature allows the bridge's error type like its siblings.
#[tauri::command]
pub async fn pick_certificate(app: AppHandle) -> Result<Option<String>, CoreError> {
    Ok(dialogs::pick_certificate(&app)
        .await
        .map(|path| path.to_string_lossy().into_owned()))
}

/// Opens the native file picker for documents (Rust owns it: the webview supplies no path) and adds
/// the chosen files to the core. Returns the added documents' ids and facts and the files
/// that were refused; never a path.
///
/// # Errors
/// `serverUnavailable` when the core is not running.
#[tauri::command]
pub async fn pick_documents(
    app: AppHandle,
    host: State<'_, HostHandle>,
) -> Result<vw_host::DocumentsPicked, CoreError> {
    let paths = dialogs::pick_documents(&app).await;
    if paths.is_empty() {
        return Ok(vw_host::DocumentsPicked {
            added: Vec::new(),
            refused: Vec::new(),
        });
    }
    host.get()
        .await
        .add_documents(paths, vw_core_api::DocumentSource::Picker)
        .await
}

/// Opens the native file picker for a library (Rust owns it: the webview names only the library) and uploads the
/// chosen files to Mistral. Returns what was added and refused; never a path.
///
/// # Errors
/// `serverUnavailable` without a core; `keyService` without a key.
#[tauri::command]
pub async fn pick_library_files(
    app: AppHandle,
    host: State<'_, HostHandle>,
    library_id: String,
) -> Result<vw_core_api::CommandResult, CoreError> {
    let paths = dialogs::pick_library_files(&app).await;
    if paths.is_empty() {
        return Ok(vw_core_api::CommandResult::Libraries {
            result: vw_core_api::LibrariesResult::Uploaded {
                added: Vec::new(),
                refused: Vec::new(),
            },
        });
    }
    host.get()
        .await
        .upload_library_paths(&library_id, paths)
        .await
}

/// "Share as one file": opens the native save dialog (Rust owns it: the webview names only the session and
/// the page) and writes the page exported as one self-contained HTML file there. `None` when the user cancelled.
///
/// # Errors
/// The core's errors (unknown session or page, a destination that cannot be written).
#[tauri::command]
pub async fn export_page(
    app: AppHandle,
    host: State<'_, HostHandle>,
    session_id: String,
    path: String,
) -> Result<Option<vw_core_api::PagesResult>, CoreError> {
    let name = vw_core_api::Core::export_name(&path);
    let Some(destination) = dialogs::save_page(&app, &name).await else {
        return Ok(None);
    };
    host.get()
        .await
        .export_page(&session_id, &path, &destination)
        .await
        .map(Some)
}

/// "Export as Markdown": opens the native save dialog (Rust owns it: the webview names only the session)
/// and writes the whole session there as Markdown. `None` when the user cancelled.
///
/// # Errors
/// The core's errors (Vibe cannot read the session, the destination cannot be written).
#[tauri::command]
pub async fn export_session(
    app: AppHandle,
    host: State<'_, HostHandle>,
    session_id: String,
) -> Result<Option<vw_core_api::SessionExportView>, CoreError> {
    let host = host.get().await;
    let name = host.export_session_name(&session_id).await;
    let Some(destination) = dialogs::save_markdown(&app, &name).await else {
        return Ok(None);
    };
    host.export_session(&session_id, &destination)
        .await
        .map(Some)
}

/// Shows a session's folder in the file manager. The host refuses any path no session works in.
///
/// # Errors
/// `denied` for an unknown folder, `internal` when the file manager cannot start.
#[tauri::command]
pub async fn reveal_folder(host: State<'_, HostHandle>, path: String) -> Result<(), CoreError> {
    host.get().await.reveal_folder(&path)
}

/// Opens a confirmed web link in the system browser. The host checks it again.
///
/// # Errors
/// `invalidParams` for a link that may not be opened, `internal` when no browser can start.
#[tauri::command]
pub async fn open_link(host: State<'_, HostHandle>, url: String) -> Result<(), CoreError> {
    host.get().await.open_link(&url)
}

/// Opens a `mailto:` link to one address in the system's mail program (Settings › About contact).
///
/// # Errors
/// `invalidParams` for anything but a plain `mailto:` link, `internal` when no opener can start.
#[tauri::command]
pub async fn open_mail_link(host: State<'_, HostHandle>, url: String) -> Result<(), CoreError> {
    host.get().await.open_mail_link(&url)
}

/// Shows an OS notification for a session; a click comes back as a host event.
///
/// # Errors
/// `invalidParams`, `denied` (rate cap) or `internal` (no notification service).
#[tauri::command]
pub async fn notify_session(
    host: State<'_, HostHandle>,
    request: NotifyRequest,
) -> Result<(), CoreError> {
    host.get().await.notify(&request)
}

/// Brings the waiting confirmation window of ask `ask_id` (an agent's request, a window of its own) to the front;
/// false when none is open. It only moves a window: nothing is answered, and the webview never sees its text.
///
/// # Errors
/// None; the signature allows the bridge's error type like its siblings.
#[tauri::command]
pub async fn show_prompt(ask_id: String) -> Result<bool, CoreError> {
    Ok(dialogs::raise_prompt(&ask_id))
}

/// Attaches the window's terminal view to a terminal's output: the scrollback, then live output, as
/// raw bytes on `output` (an `ArrayBuffer` per chunk in JS; no JSON number arrays, ShellX
/// `terminal.rs:658-668`). Flow control: the view acknowledges rendered bytes with `terminal_ack`.
///
/// # Errors
/// `notFound` for an unknown terminal.
#[tauri::command]
pub async fn terminal_attach(
    host: State<'_, HostHandle>,
    terminal_id: String,
    output: Channel<InvokeResponseBody>,
) -> Result<TerminalAttached, CoreError> {
    host.get()
        .await
        .terminal_attach(&terminal_id, move |bytes| {
            output.send(InvokeResponseBody::Raw(bytes.to_vec())).is_ok()
        })
}

/// The view rendered `bytes` more bytes of the terminal (flow control).
///
/// # Errors
/// `serverUnavailable` without a core.
#[tauri::command]
pub async fn terminal_ack(
    host: State<'_, HostHandle>,
    terminal_id: String,
    viewer: u64,
    bytes: u64,
) -> Result<(), CoreError> {
    host.get().await.terminal_ack(&terminal_id, viewer, bytes)
}

/// The view stopped showing the terminal; it keeps running and buffering.
///
/// # Errors
/// `serverUnavailable` without a core.
#[tauri::command]
pub async fn terminal_detach(
    host: State<'_, HostHandle>,
    terminal_id: String,
    viewer: u64,
) -> Result<(), CoreError> {
    host.get().await.terminal_detach(&terminal_id, viewer)
}

/// The manager, or the error every `update_*` command returns in a build without an update channel.
fn updates(handle: &UpdatesHandle) -> Result<&UpdateManager, CoreError> {
    handle.get().ok_or_else(|| {
        CoreError::new(
            ErrorCode::Denied,
            "this build has no update channel".to_owned(),
        )
    })
}

/// Starts the stream of update views into `events` (the current one first, then every change) and says whether
/// this build has an update channel at all (`false`: the UI shows no update section).
///
/// # Errors
/// None today; the result type is the bridge's.
#[tauri::command]
pub async fn update_subscribe(
    handle: State<'_, UpdatesHandle>,
    events: Channel<UpdateView>,
) -> Result<bool, CoreError> {
    let Some(manager) = handle.get() else {
        return Ok(false);
    };
    let mut receiver = manager.subscribe();
    tauri::async_runtime::spawn(async move {
        let first = receiver.borrow_and_update().clone();
        if events.send(first).is_err() {
            return;
        }
        while receiver.changed().await.is_ok() {
            let view = receiver.borrow_and_update().clone();
            if events.send(view).is_err() {
                break;
            }
        }
    });
    Ok(true)
}

/// Checks for a newer version now ("Check now", "Try again"). Returns when the check, and the download it
/// starts in the automatic mode, has finished; progress arrives on the `update_subscribe` stream.
///
/// # Errors
/// `denied` without a channel or while another update step runs.
#[tauri::command]
pub async fn update_check(handle: State<'_, UpdatesHandle>) -> Result<(), CoreError> {
    updates(&handle)?.check().await.map_err(CoreError::from)
}

/// Downloads the offered update (the "ask before downloading" mode, or a retry). A `ui/updates download` request is
/// carried out through this same window command: the request is the Debug API's agreement as the click is the user's.
///
/// # Errors
/// `denied`, `confirmationDeclined` (dialog declined), `internal` (the download failed).
#[tauri::command]
pub async fn update_download(handle: State<'_, UpdatesHandle>) -> Result<(), CoreError> {
    updates(&handle)?
        .download(UpdateRequester::Window)
        .await
        .map_err(CoreError::from)
}

/// "Install and restart": records the user's agreement and installs. The click is the agreement (a
/// `ui/updates install` request clicks it). The core installs nothing without a recorded agreement
/// (`vw_host::update_flow`).
///
/// # Errors
/// `denied` (nothing ready, busy, no agreement), `confirmationDeclined`, `internal` (the install failed).
#[tauri::command]
pub async fn update_install(handle: State<'_, UpdatesHandle>) -> Result<(), CoreError> {
    updates(&handle)?
        .request_install(UpdateRequester::Window)
        .await
        .map_err(CoreError::from)
}

/// Saves the "Download updates automatically" switch.
///
/// # Errors
/// `denied` without a channel, `internal` when the setting cannot be saved.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri hands `State` over by value
pub async fn update_set_auto_download(
    handle: State<'_, UpdatesHandle>,
    enabled: bool,
) -> Result<(), CoreError> {
    updates(&handle)?
        .set_auto_download(enabled)
        .map_err(CoreError::from)
}

/// Opens the offered version's release page in the system browser: always the official repository's tag for that
/// version (`vw_host::update_flow::release_page_for`), never a link from the update manifest.
///
/// # Errors
/// `denied` when there is no link, `invalidParams`/`internal` from the browser launch.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri hands `State` over by value
pub async fn update_open_release_page(handle: State<'_, UpdatesHandle>) -> Result<(), CoreError> {
    let Some(url) = updates(&handle)?.view().release_page else {
        return Err(CoreError::new(
            ErrorCode::Denied,
            "this update has no release page".to_owned(),
        ));
    };
    vw_platform::open_web_url(&url)
        .map_err(|error| CoreError::new(ErrorCode::InvalidParams, error.to_string()))
}

/// Tells the window (an empty message) whenever hand-overs wait, at once when some already do. A second call
/// replaces the first (a reload).
#[tauri::command]
pub async fn hand_over_subscribe(
    inbox: State<'_, std::sync::Arc<vw_host::hand_over::HandOverInbox>>,
    events: Channel<()>,
) -> Result<(), CoreError> {
    inbox.subscribe(move || events.send(()).is_ok());
    Ok(())
}

/// Takes every waiting hand-over, oldest first.
#[tauri::command]
pub async fn hand_over_take(
    host: State<'_, HostHandle>,
    inbox: State<'_, std::sync::Arc<vw_host::hand_over::HandOverInbox>>,
) -> Result<Vec<vw_host::hand_over::HandOverView>, CoreError> {
    host.get().await.take_hand_overs(&inbox).await
}

/// Reads or changes the "Right-click menus" switch, or answers the one-time question.
///
/// # Errors
/// `internal` with the sentence the window shows when an entry could not be written or removed.
#[tauri::command]
pub async fn send_to(
    host: State<'_, HostHandle>,
    request: vw_host::send_to::SendToRequest,
) -> Result<vw_host::send_to::SendToView, CoreError> {
    let host = host.get().await;
    tauri::async_runtime::spawn_blocking(move || host.send_to(request))
        .await
        .map_err(|error| CoreError::new(ErrorCode::Internal, error.to_string()))?
}
