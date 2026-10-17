//! Libraries and image generation in the core: the
//! `libraries` and `images` commands, the saved choices (which library each project may use) and the pieces the
//! agent's tools share (`agent_tools::work_tools`).
//!
//! Mistral is called by the key service with the key in use (vw-keys `work`), never through the webview. Documents
//! live in the user's Mistral account (the Libraries API); Vibe Work keeps only `work.json` (user-only): library id ->
//! scope. Image generation asks nothing of its own (Vibe's card is the one ask); `images` reads the use
//! facts and this run's request count for Settings.
//!
//! Depends on: vw-keys (`KeyService` work calls), vw-mistral (`work`), vw-projects (a scope's project), vw-platform
//! (user-only file), `work_view` (wire types). Called by: `dispatch` (`Command::Libraries`, `Command::Images`),
//! `Core::upload_library_paths` (the native picker), `agent_tools::work_tools`.
//! Invariants:
//! - Both clients act alike (the Debug API acts as the user). No request names a path; the picker's paths
//!   come from the OS (`upload_library_paths`).
//! - A library the user did not switch on (scope `off`, also every library made elsewhere) is never searched by an
//!   agent.
//! - Logs name ids, counts, sizes and token totals only: never a question, an answer, a prompt or file text.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use vw_keys::{KeyService, WorkFailure};
use vw_mistral::work::images::{FetchPolicy, Timeouts};
use vw_mistral::work::libraries::{Library, LibraryDocument, MAX_UPLOAD_BYTES};

use crate::command::CommandResult;
use crate::core::{Core, Shared};
use crate::error::{CoreError, ErrorCode};
use crate::work_view::{
    ImagesRequest, ImagesResult, LibrariesRequest, LibrariesResult, LibraryDocumentView,
    LibraryFile, LibraryScope, LibrarySourceView, LibraryUploadRefusal, LibraryView, WorkCosts,
};

/// Largest file sent as bytes in a request (`upload`); the picker path allows Mistral's 100 MB.
pub const MAX_REQUEST_FILE_BYTES: usize = 50 * 1024 * 1024;
/// Longest question.
pub const MAX_QUESTION_CHARS: usize = 4_000;

/// Settings of Libraries and image generation.
#[derive(Debug, Clone)]
pub struct WorkConfig {
    /// Folder of `work.json` (user-only); `None`: kept in memory for this run (tests).
    pub dir: Option<PathBuf>,
    /// The model that runs library searches and writes the answer.
    pub library_model: String,
    /// The model that drives the image tool.
    pub image_model: String,
    /// Where generated images may be downloaded from.
    pub image_fetch: FetchPolicy,
    /// Time limits of one image request.
    pub image_timeouts: Timeouts,
    /// The whole image tool call must finish within this (under the agent tool's 50 s wire limit).
    pub image_deadline: Duration,
}

impl Default for WorkConfig {
    fn default() -> Self {
        Self {
            dir: None,
            // Mistral Medium (GA alias) runs the built-in tools in Mistral's own docs; Large 4 is a preview model.
            library_model: "mistral-medium-latest".to_owned(),
            image_model: "mistral-medium-latest".to_owned(),
            image_fetch: FetchPolicy::default(),
            image_timeouts: Timeouts::default(),
            image_deadline: Duration::from_secs(45),
        }
    }
}

/// What `work.json` holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Saved {
    /// Library id -> scope (missing: off). An `imagesAllowed` key from before is ignored.
    libraries: BTreeMap<String, LibraryScope>,
}

/// The core's work state: saved choices and this run's image counters.
pub(crate) struct WorkState {
    file: Option<PathBuf>,
    saved: Mutex<Saved>,
    /// Image requests sent to Mistral this run (all sessions; failed ones too).
    pub(crate) made: AtomicU32,
    /// One image request at a time, across all sessions.
    pub(crate) image_lock: tokio::sync::Mutex<()>,
}

impl WorkState {
    /// Loads `work.json` from `config.dir` (a missing or unreadable file starts empty, logged).
    pub(crate) fn open(config: &WorkConfig) -> Self {
        let file = config.dir.as_ref().map(|d| d.join("work.json"));
        let saved = file
            .as_deref()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|bytes| {
                serde_json::from_slice::<Saved>(&bytes)
                    .map_err(|e| warn!(error = %e, "work.json could not be read; starting empty"))
                    .ok()
            })
            .unwrap_or_default();
        Self {
            file,
            saved: Mutex::new(saved),
            made: AtomicU32::new(0),
            image_lock: tokio::sync::Mutex::new(()),
        }
    }

    fn read<T>(&self, f: impl FnOnce(&Saved) -> T) -> T {
        f(&self.saved.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Changes the saved state and writes it (user-only, atomic replace).
    fn change(&self, f: impl FnOnce(&mut Saved)) -> Result<(), CoreError> {
        let mut saved = self.saved.lock().unwrap_or_else(PoisonError::into_inner);
        let mut next = saved.clone();
        f(&mut next);
        if let Some(file) = &self.file {
            if let Some(dir) = file.parent() {
                vw_platform::ensure_private_dir(dir)
                    .map_err(|e| CoreError::new(ErrorCode::Internal, e.to_string()))?;
            }
            let bytes = serde_json::to_vec_pretty(&next)
                .map_err(|e| CoreError::new(ErrorCode::Internal, e.to_string()))?;
            vw_platform::replace_private(file, &bytes)
                .map_err(|e| CoreError::new(ErrorCode::Internal, e.to_string()))?;
        }
        *saved = next;
        Ok(())
    }

    /// The scope of a library (off when never set).
    pub(crate) fn scope_of(&self, id: &str) -> LibraryScope {
        self.read(|s| s.libraries.get(id).cloned())
            .unwrap_or(LibraryScope::Off {})
    }

    /// The libraries an agent of a session in `project` may search.
    pub(crate) fn searchable_for(&self, project: Option<&str>) -> Vec<String> {
        self.read(|s| {
            s.libraries
                .iter()
                .filter(|(_, scope)| match scope {
                    LibraryScope::Off {} => false,
                    LibraryScope::Global {} => true,
                    LibraryScope::Project { project_id } => Some(project_id.as_str()) == project,
                })
                .map(|(id, _)| id.clone())
                .collect()
        })
    }
}

/// What Libraries sends and its request limits.
#[must_use]
pub fn library_costs() -> WorkCosts {
    WorkCosts {
        leaves: "Files you add are sent to Mistral and kept in your Mistral account (stored in the EU) until you \
                 delete them here or in Mistral's console. A question sends the question and the matching passages \
                 through Mistral's models."
            .to_owned(),
        limits: "Up to 100 MB per file.".to_owned(),
    }
}

/// What image generation sends and the app's own request and file limits.
#[must_use]
pub fn image_costs() -> WorkCosts {
    WorkCosts {
        leaves: "Your prompt, and an image you ask to change, go to Mistral; the image is made on Mistral's service \
                 (a Black Forest Labs model) and saved in the session's folder under images/."
            .to_owned(),
        limits: "One image request at a time. Edit sources up to 10 MB; downloaded images up to 25 MB.".to_owned(),
    }
}

pub(crate) fn keys(shared: &Shared) -> Result<Arc<KeyService>, CoreError> {
    shared.config.keys.clone().ok_or_else(|| {
        let mut error = CoreError::new(
            ErrorCode::KeyService,
            "this run has no key service, so Vibe Work cannot reach Mistral for Libraries or images",
        );
        error.server_code = Some("notConfigured".to_owned());
        error
    })
}

/// A failed Mistral call as a core error (`keyService` + `keyNeeded`, or `serverError` + `work`).
pub(crate) fn failure(error: &WorkFailure) -> CoreError {
    let (code, server) = match error {
        WorkFailure::KeyNeeded(_) => (ErrorCode::KeyService, "keyNeeded"),
        WorkFailure::Work(vw_mistral::work::WorkError::InvalidParams(_)) => {
            (ErrorCode::InvalidParams, "input")
        }
        WorkFailure::Work(_) => (ErrorCode::ServerError, "work"),
    };
    let mut e = CoreError::new(code, error.to_string());
    e.server_code = Some(server.to_owned());
    e
}

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::new(ErrorCode::InvalidParams, message)
}

pub(crate) fn library_view(library: Library, scope: LibraryScope) -> LibraryView {
    LibraryView {
        id: library.id,
        name: library.name,
        description: library.description,
        documents: library.documents,
        total_size: library.total_size,
        updated_at: library.updated_at,
        scope,
    }
}

pub(crate) fn document_view(d: LibraryDocument) -> LibraryDocumentView {
    LibraryDocumentView {
        id: d.id,
        library_id: d.library_id,
        name: d.name,
        size: d.size,
        status: d.status,
        pages: d.pages,
        summary: d.summary,
        tokens: d.tokens,
        created_at: d.created_at,
    }
}

fn clean_name(name: &str) -> Result<String, CoreError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 100 || name.chars().any(char::is_control) {
        return Err(invalid(
            "a library name is 1 to 100 characters, on one line",
        ));
    }
    Ok(name.to_owned())
}

fn check_scope(shared: &Shared, scope: &LibraryScope) -> Result<(), CoreError> {
    if let LibraryScope::Project { project_id } = scope {
        let known = shared
            .projects
            .store
            .read(|s| s.project(project_id).is_some());
        if !known {
            return Err(CoreError::new(
                ErrorCode::NotFound,
                "no such project (it was deleted?)",
            ));
        }
    }
    Ok(())
}

/// The MIME type sent with an upload, by extension (Mistral checks the content itself).
#[must_use]
pub fn mime_of(name: &str) -> &'static str {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "odt" => "application/vnd.oasis.opendocument.text",
        "epub" => "application/epub+zip",
        "rtf" => "application/rtf",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "md" | "markdown" | "txt" | "log" | "yaml" | "yml" | "toml" | "xml" => "text/plain",
        _ => "application/octet-stream",
    }
}

/// The last path segment of a client-given file name (a name cannot carry a folder).
fn last_segment(name: &str) -> String {
    vw_mistral::work::header_file_name(name)
}

/// Uploads `(name, bytes)` pairs one by one; a failure refuses that file and the next one is still tried.
async fn upload_all(
    keys: &KeyService,
    library: &str,
    files: Vec<(String, Result<Vec<u8>, String>)>,
) -> Result<CommandResult, CoreError> {
    let mut added = Vec::new();
    let mut refused = Vec::new();
    for (name, bytes) in files {
        let bytes = match bytes {
            Ok(b) if b.is_empty() => Err("the file is empty".to_owned()),
            other => other,
        };
        match bytes {
            Err(reason) => refused.push(LibraryUploadRefusal { name, reason }),
            Ok(bytes) => match keys
                .library_upload(library, (&name, mime_of(&name), &bytes))
                .await
            {
                Ok(doc) => {
                    info!(library, document = %doc.id, bytes = bytes.len(), "library upload");
                    added.push(document_view(doc));
                }
                Err(WorkFailure::KeyNeeded(need)) => {
                    return Err(failure(&WorkFailure::KeyNeeded(need)));
                }
                Err(error) => refused.push(LibraryUploadRefusal {
                    name,
                    reason: error.to_string(),
                }),
            },
        }
    }
    Ok(CommandResult::Libraries {
        result: LibrariesResult::Uploaded { added, refused },
    })
}

fn decode(file: &LibraryFile) -> (String, Result<Vec<u8>, String>) {
    let name = last_segment(&file.name);
    // Base64 is 4/3 of the bytes: refuse an over-long text before decoding it.
    if file.data.len() > MAX_REQUEST_FILE_BYTES / 3 * 4 + 4 {
        return (
            name,
            Err("larger than 50 MB (use Add files for larger files)".to_owned()),
        );
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(file.data.as_bytes())
        .map_err(|_| "the file data is not base64".to_owned());
    (name, bytes)
}

/// Which searched library holds each source (one documents read per library).
async fn place_sources(
    keys: &KeyService,
    libraries: &[String],
    sources: Vec<vw_mistral::work::libraries::Source>,
) -> Vec<LibrarySourceView> {
    let mut owner: HashMap<String, String> = HashMap::new();
    for library in libraries {
        if let Ok(docs) = keys.library_documents(library).await {
            for doc in docs {
                owner.entry(doc.id).or_insert_with(|| library.clone());
            }
        }
    }
    sources
        .into_iter()
        .map(|s| LibrarySourceView {
            n: s.n,
            library_id: owner.get(&s.document_id).cloned(),
            document_id: s.document_id,
            title: s.title,
            snippet: s.snippet,
        })
        .collect()
}

/// Sums every token count of a usage object.
#[must_use]
pub fn tokens_of(usage: &serde_json::Value) -> u64 {
    ["prompt_tokens", "completion_tokens", "connector_tokens"]
        .iter()
        .filter_map(|k| usage.get(*k).and_then(serde_json::Value::as_u64))
        .sum()
}

/// Asks across `libraries` and builds the answer (shared by the `ask` command and the agent's `library_search`).
pub(crate) async fn ask(
    shared: &Shared,
    question: &str,
    libraries: Vec<String>,
) -> Result<LibrariesResult, CoreError> {
    let question = question.trim();
    if question.is_empty() || question.chars().count() > MAX_QUESTION_CHARS {
        return Err(invalid(format!(
            "a question is 1 to {MAX_QUESTION_CHARS} characters"
        )));
    }
    if libraries.is_empty() {
        return Err(invalid(
            "no library to search: switch a library on for all chats or for this project in Libraries",
        ));
    }
    let keys = keys(shared)?;
    let answer = keys
        .library_search(&shared.config.work.library_model, question, &libraries)
        .await
        .map_err(|e| failure(&e))?;
    let tokens = tokens_of(&answer.usage);
    info!(
        libraries = libraries.len(),
        sources = answer.sources.len(),
        searches = answer.queries.len(),
        tokens,
        model = %answer.model,
        "library search"
    );
    let sources = place_sources(&keys, &libraries, answer.sources).await;
    Ok(LibrariesResult::Answer {
        answer: answer.answer,
        sources,
        queries: answer.queries,
        library_ids: libraries,
        tokens,
    })
}

/// Runs a `libraries` request.
pub(crate) async fn libraries(
    shared: &Shared,
    request: LibrariesRequest,
) -> Result<CommandResult, CoreError> {
    validate_library_ids(&request)?;
    let keys = keys(shared)?;
    let result = match request {
        LibrariesRequest::List {} => {
            let list = keys.libraries().await.map_err(|e| failure(&e))?;
            if let Some(message) = &list.partial {
                shared.emit(crate::event::CoreEvent::Alert {
                    session_id: None,
                    kind: crate::event::AlertKind::LibraryListPartial,
                    message: message.clone(),
                });
            }
            LibrariesResult::List {
                libraries: list
                    .libraries
                    .into_iter()
                    .map(|l| {
                        let scope = shared.work.scope_of(&l.id);
                        library_view(l, scope)
                    })
                    .collect(),
                costs: library_costs(),
                partial: list.partial,
            }
        }
        LibrariesRequest::Upload { library_id, files } => {
            if files.is_empty() || files.len() > 100 {
                return Err(invalid("upload 1 to 100 files at once"));
            }
            let decoded = files.iter().map(decode).collect();
            return upload_all(&keys, &library_id, decoded).await;
        }
        LibrariesRequest::Ask {
            question,
            library_ids,
        } => {
            let libraries = if library_ids.is_empty() {
                shared.work.read(|s| {
                    s.libraries
                        .iter()
                        .filter(|(_, scope)| !matches!(scope, LibraryScope::Off {}))
                        .map(|(id, _)| id.clone())
                        .collect()
                })
            } else {
                library_ids
            };
            ask(shared, &question, libraries).await?
        }
        LibrariesRequest::Documents { .. }
        | LibrariesRequest::DeleteDocument { .. }
        | LibrariesRequest::Text { .. } => documents(&keys, request).await?,
        other => manage(shared, &keys, other).await?,
    };
    Ok(CommandResult::Libraries { result })
}

/// Caller ids are checked before reading keys, requesting grants, or sending provider work.
fn validate_library_ids(request: &LibrariesRequest) -> Result<(), CoreError> {
    let check = |id: &str| {
        vw_mistral::work::libraries::id_segment(id)
            .map(|_| ())
            .map_err(|error| invalid(error.to_string()))
    };
    match request {
        LibrariesRequest::SetScope { library_id, .. }
        | LibrariesRequest::Delete { library_id }
        | LibrariesRequest::Documents { library_id }
        | LibrariesRequest::Upload { library_id, .. } => check(library_id),
        LibrariesRequest::DeleteDocument {
            library_id,
            document_id,
        }
        | LibrariesRequest::Text {
            library_id,
            document_id,
        } => {
            check(library_id)?;
            check(document_id)
        }
        LibrariesRequest::Ask { library_ids, .. } => {
            for id in library_ids {
                check(id)?;
            }
            Ok(())
        }
        LibrariesRequest::List {} | LibrariesRequest::Create { .. } => Ok(()),
    }
}

/// `create`, `setScope`, `delete`.
async fn manage(
    shared: &Shared,
    keys: &KeyService,
    request: LibrariesRequest,
) -> Result<LibrariesResult, CoreError> {
    let work = &shared.work;
    let fail = |e: WorkFailure| failure(&e);
    Ok(match request {
        LibrariesRequest::Create {
            name,
            description,
            scope,
        } => {
            let name = clean_name(&name)?;
            check_scope(shared, &scope)?;
            let description = description
                .map(|d| d.trim().chars().take(500).collect::<String>())
                .filter(|d| !d.is_empty());
            let library = keys
                .library_create(&name, description.as_deref())
                .await
                .map_err(fail)?;
            let id = library.id.clone();
            work.change(|s| {
                s.libraries.insert(id, scope.clone());
            })?;
            info!(library = %library.id, "library created");
            LibrariesResult::Library {
                library: library_view(library, scope),
            }
        }
        LibrariesRequest::SetScope { library_id, scope } => {
            check_scope(shared, &scope)?;
            let list = keys.libraries().await.map_err(fail)?;
            let partial = list.partial;
            let library = list
                .libraries
                .into_iter()
                .find(|l| l.id == library_id)
                .ok_or_else(|| match partial {
                    Some(message) => CoreError::new(ErrorCode::ServerError, message),
                    None => CoreError::new(ErrorCode::NotFound, "no such library"),
                })?;
            work.change(|s| {
                s.libraries.insert(library_id.clone(), scope.clone());
            })?;
            LibrariesResult::Library {
                library: library_view(library, scope),
            }
        }
        LibrariesRequest::Delete { library_id } => {
            keys.library_delete(&library_id).await.map_err(fail)?;
            work.change(|s| {
                s.libraries.remove(&library_id);
            })?;
            info!(library = %library_id, "library deleted");
            LibrariesResult::Deleted {}
        }
        _ => return Err(CoreError::new(ErrorCode::Internal, "not a manage request")),
    })
}

/// `documents`, `deleteDocument`, `text`.
async fn documents(
    keys: &KeyService,
    request: LibrariesRequest,
) -> Result<LibrariesResult, CoreError> {
    let fail = |e: WorkFailure| failure(&e);
    Ok(match request {
        LibrariesRequest::Documents { library_id } => {
            let documents = keys
                .library_documents(&library_id)
                .await
                .map_err(fail)?
                .into_iter()
                .map(document_view)
                .collect();
            LibrariesResult::Documents {
                library_id,
                documents,
            }
        }
        LibrariesRequest::DeleteDocument {
            library_id,
            document_id,
        } => {
            keys.library_document_delete(&library_id, &document_id)
                .await
                .map_err(fail)?;
            LibrariesResult::Deleted {}
        }
        LibrariesRequest::Text {
            library_id,
            document_id,
        } => {
            let name = keys
                .library_documents(&library_id)
                .await
                .map_err(fail)?
                .into_iter()
                .find(|d| d.id == document_id)
                .map(|d| d.name)
                .ok_or_else(|| CoreError::new(ErrorCode::NotFound, "no such document"))?;
            let (text, truncated) = keys
                .library_document_text(&library_id, &document_id)
                .await
                .map_err(fail)?;
            LibrariesResult::Text {
                name,
                text,
                truncated,
            }
        }
        _ => {
            return Err(CoreError::new(
                ErrorCode::Internal,
                "not a documents request",
            ));
        }
    })
}

/// Runs an `images` request (the only op reads: the use facts and this run's request count).
pub(crate) fn images(shared: &Shared, request: &ImagesRequest) -> CommandResult {
    let ImagesRequest::Read {} = request;
    CommandResult::Images {
        result: ImagesResult {
            costs: image_costs(),
            made: shared.work.made.load(Ordering::Relaxed),
        },
    }
}

impl Core {
    /// Uploads files the native picker gave (Rust side; the webview names only the library) into a library. Files
    /// over 100 MB, folders and unreadable files are refused with a reason.
    ///
    /// # Errors
    /// `keyService` without a key service or key; other failures are per-file refusals.
    pub async fn upload_library_paths(
        &self,
        library_id: &str,
        paths: Vec<PathBuf>,
    ) -> Result<CommandResult, CoreError> {
        let keys = keys(&self.shared)?;
        let files = tokio::task::spawn_blocking(move || {
            paths
                .into_iter()
                .map(|path| {
                    let name = last_segment(&path.to_string_lossy());
                    let bytes = match std::fs::metadata(&path) {
                        Ok(m) if !m.is_file() => Err("not a file".to_owned()),
                        Ok(m) if m.len() > MAX_UPLOAD_BYTES as u64 => {
                            Err("larger than 100 MB (Mistral's limit per file)".to_owned())
                        }
                        Ok(_) => {
                            std::fs::read(&path).map_err(|e| format!("could not be read: {e}"))
                        }
                        Err(e) => Err(format!("could not be read: {e}")),
                    };
                    (name, bytes)
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|e| CoreError::new(ErrorCode::Internal, format!("task failed: {e}")))?;
        upload_all(&keys, library_id, files).await
    }
}
