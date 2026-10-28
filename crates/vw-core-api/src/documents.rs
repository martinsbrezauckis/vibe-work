//! The `documents` command and how files enter the core.
//!
//! Files enter through [`Core::add_documents`] for native picker paths, or `addDropped` for
//! bytes read by the window on drop/paste (origin `ui`; native Tauri drops are disabled). The
//! `documents` command then lists, reads, removes and extracts them; extraction calls Mistral OCR from
//! the core with the key from the key service, never through the webview.
//!
//! Depends on: vw-documents (store, kinds), vw-keys (`KeyService::ocr`), vw-mistral (OCR input),
//! `documents_view`, `core` (`Shared`). Called by: `dispatch`, vw-host (adds).
//! Invariants:
//! - Every origin runs `extract` directly (the Debug API acts as the user; its dialog is gone).
//! - Logs name the document id, kind, size and outcome only: never its text, never the key.
//! - Every change of a document is published as `documentChanged` (and additions as `documentsAdded`).

use std::path::PathBuf;
use std::time::Duration;

use base64::Engine as _;
use tracing::{info, warn};
use vw_documents::{DocStatus, OcrPages};

use crate::command::CommandResult;
use crate::core::{Core, Shared};
use crate::documents_view::{
    DocumentRefusal, DocumentSource, DocumentView, DocumentsRequest, DocumentsResult,
};
use crate::error::{CoreError, ErrorCode};
use crate::event::CoreEvent;

/// Document settings.
#[derive(Debug, Clone)]
pub struct DocumentsConfig {
    /// Longest wait for one OCR request (default [`vw_mistral::OCR_TIMEOUT`], 300 s).
    pub ocr_timeout: Duration,
    /// Maximum hold of an agent tool call before returning a resumable reading status.
    pub agent_wait: Duration,
}

impl Default for DocumentsConfig {
    fn default() -> Self {
        Self {
            ocr_timeout: vw_mistral::OCR_TIMEOUT,
            agent_wait: Duration::from_secs(25),
        }
    }
}

fn result(result: DocumentsResult) -> CommandResult {
    CommandResult::Documents { result }
}

fn not_found(id: &str) -> CoreError {
    CoreError::new(
        ErrorCode::NotFound,
        format!("no document {id} (it was removed or sent)"),
    )
}

fn ocr_pages(out: vw_mistral::OcrOutput) -> Result<OcrPages, String> {
    if out.pages.iter().all(|page| page.trim().is_empty()) {
        return Err(
            "No readable text was found in this document. Try another copy of the file.".to_owned(),
        );
    }
    Ok(OcrPages {
        pages: out.pages,
        model: out.model,
        pages_processed: out.pages_processed,
    })
}

/// Runs OCR on one document and stores the outcome (the command waits for it).
async fn extract(shared: &Shared, id: &str) -> Result<CommandResult, CoreError> {
    let store = &shared.documents;
    store.info(id).ok_or_else(|| not_found(id))?;
    let Some(keys) = shared.config.keys.clone() else {
        let mut error = CoreError::new(
            ErrorCode::KeyService,
            "this run has no key service, so documents cannot be read with OCR",
        );
        error.server_code = Some("notConfigured".to_owned());
        return Err(error);
    };
    let job = store
        .begin_read(id)
        .map_err(|e| CoreError::new(ErrorCode::InvalidParams, e.to_string()))?;
    if let Some(reading) = store.info(id) {
        shared.emit(CoreEvent::DocumentChanged {
            document: DocumentView::from(&reading),
        });
    }
    info!(document = id, kind = ?job.kind, bytes = job.bytes.len(), "OCR started");
    let input = vw_mistral::OcrInput {
        bytes: &job.bytes,
        mime: job.kind.mime(),
        image: job.kind.is_image(),
    };
    let outcome = keys
        .ocr(&input, shared.config.documents.ocr_timeout)
        .await
        .map_err(|e| e.to_string())
        .and_then(ocr_pages);
    match &outcome {
        Ok(pages) => info!(document = id, pages = pages.pages.len(), "OCR finished"),
        Err(reason) => warn!(document = id, %reason, "OCR failed"),
    }
    let Some(done) = store.finish_read(job, outcome) else {
        return Err(not_found(id));
    };
    let document = DocumentView::from(&done);
    shared.emit(CoreEvent::DocumentChanged {
        document: document.clone(),
    });
    let text = if done.status == DocStatus::Ready {
        store.read(id).and_then(|(_, text)| text)
    } else {
        None
    };
    if let Some(text) = &text {
        shared.saved_documents.seed(
            id,
            &crate::saved_documents::Text {
                text: store.full_text(id).unwrap_or_else(|| text.clone()),
                kept: document.kept,
            },
        );
    }
    Ok(result(DocumentsResult::Document { document, text }))
}

/// Most files in one `addDropped` (the store holds at most `vw_documents::MAX_DOCS` waiting ones anyway).
const MAX_DROPPED: usize = 32;

/// The last segment of a dropped file's name (no folder), at most 255 characters, never empty.
fn dropped_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    let clean: String = last.chars().filter(|c| !c.is_control()).take(255).collect();
    if clean.is_empty() {
        "file".to_owned()
    } else {
        clean
    }
}

/// `addDropped`: bytes the window read from files the user dropped on a composer.
async fn add_dropped(
    shared: &std::sync::Arc<Shared>,
    files: Vec<crate::documents_view::DroppedFile>,
) -> Result<CommandResult, CoreError> {
    if files.len() > MAX_DROPPED {
        return Err(CoreError::new(
            ErrorCode::InvalidParams,
            format!("at most {MAX_DROPPED} files can be dropped at once"),
        ));
    }
    // Base64 of the largest accepted file, plus padding: anything longer is refused before decoding.
    let max_encoded =
        usize::try_from(vw_documents::MAX_FILE_BYTES.div_ceil(3) * 4).unwrap_or(usize::MAX);
    let mut added = Vec::new();
    let mut refused = Vec::new();
    for file in files {
        let name = dropped_name(&file.name);
        if file.data.len() > max_encoded {
            refused.push(DocumentRefusal {
                name,
                reason: vw_documents::AddError::TooLarge.to_string(),
            });
            continue;
        }
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(file.data.as_bytes())
        else {
            refused.push(DocumentRefusal {
                name,
                reason: "the file could not be read".to_owned(),
            });
            continue;
        };
        let core_shared = shared.clone();
        let file_name = name.clone();
        let outcome =
            tokio::task::spawn_blocking(move || core_shared.documents.add_bytes(&file_name, bytes))
                .await
                .map_err(|e| {
                    CoreError::new(ErrorCode::Internal, format!("document task failed: {e}"))
                })?;
        match outcome {
            Ok(info) => {
                info!(document = %info.id, kind = ?info.kind, bytes = info.size, "document dropped");
                added.push(DocumentView::from(&info));
            }
            Err(error) => refused.push(DocumentRefusal {
                name,
                reason: error.to_string(),
            }),
        }
    }
    shared.emit(CoreEvent::DocumentsAdded {
        via: DocumentSource::Composer,
        added: added.clone(),
        refused: refused.clone(),
    });
    Ok(result(DocumentsResult::Added { added, refused }))
}

/// Executes one documents request.
///
/// # Errors
/// `notFound` for an unknown id; `invalidParams` for a request the document's state does not allow;
/// `keyService` without a key service. A failed OCR is not an error: the
/// document comes back `failed` with the reason.
pub(crate) async fn execute(
    shared: &std::sync::Arc<Shared>,
    request: DocumentsRequest,
) -> Result<CommandResult, CoreError> {
    let store = &shared.documents;
    match request {
        DocumentsRequest::List {} => Ok(result(DocumentsResult::List {
            documents: store.list().iter().map(DocumentView::from).collect(),
        })),
        DocumentsRequest::Read { id } => {
            let (info, text) = store.read(&id).ok_or_else(|| not_found(&id))?;
            Ok(result(DocumentsResult::Document {
                document: DocumentView::from(&info),
                text,
            }))
        }
        DocumentsRequest::Save { id, session_id } => {
            let cwd = shared
                .names
                .cwd_of(&shared.store, &session_id)
                .ok_or_else(|| {
                    CoreError::new(
                        ErrorCode::NotFound,
                        "attachment session has no project folder",
                    )
                })?;
            let owned = shared.clone();
            let document_id = id.clone();
            let path = tokio::task::spawn_blocking(move || {
                owned.documents.save_with(&id, &session_id, |info, bytes| {
                    let path = crate::document_save::save(
                        std::path::Path::new(&cwd),
                        &session_id,
                        &info.id,
                        &info.name,
                        bytes,
                    )?;
                    let root = vw_platform::canonical_plain(std::path::Path::new(&cwd))
                        .map_err(|e| e.to_string())?;
                    let relative = path
                        .strip_prefix(&root)
                        .map_err(|e| e.to_string())?
                        .to_path_buf();
                    owned.saved_documents.register(
                        &owned,
                        &session_id,
                        &info.id,
                        relative,
                        crate::saved_documents::digest(bytes),
                    )?;
                    Ok(path)
                })
            })
            .await
            .map_err(|e| {
                CoreError::new(ErrorCode::Internal, format!("attachment task failed: {e}"))
            })?
            .map_err(|e| CoreError::new(ErrorCode::InvalidParams, e.to_string()))?;
            if let Some((info, Some(text))) = store.read(&document_id) {
                shared.saved_documents.seed(
                    &document_id,
                    &crate::saved_documents::Text {
                        text: store.full_text(&document_id).unwrap_or(text),
                        kept: DocumentView::from(&info).kept,
                    },
                );
            }
            Ok(result(DocumentsResult::Saved {
                path: path.to_string_lossy().into_owned(),
            }))
        }
        DocumentsRequest::Extract { id } => extract(shared, &id).await,
        DocumentsRequest::AddDropped { files } => add_dropped(shared, files).await,
        DocumentsRequest::Release { id } => Ok(result(DocumentsResult::Removed {
            removed: store.remove(&id),
        })),
        DocumentsRequest::Remove { id } => {
            let removed = store.remove(&id);
            let revoked = shared.saved_documents.remove(&id);
            Ok(result(DocumentsResult::Removed {
                removed: removed || revoked,
            }))
        }
        DocumentsRequest::TakePicture { id } => {
            let info = store.info(&id).ok_or_else(|| not_found(&id))?;
            let (mime, bytes) = store
                .picture(&id)
                .map_err(|e| CoreError::new(ErrorCode::InvalidParams, e.to_string()))?;
            Ok(result(DocumentsResult::Picture {
                mime_type: mime.to_owned(),
                data: base64::engine::general_purpose::STANDARD.encode(bytes),
                name: info.name,
            }))
        }
    }
}

impl Core {
    /// Adds the files the native picker or an OS drop gave (never a path from the webview, the Debug
    /// API or the model) and publishes `documentsAdded`. Each file is either
    /// added or refused with the reason; reading runs off the async runtime.
    pub async fn add_documents(
        &self,
        paths: Vec<PathBuf>,
        via: DocumentSource,
    ) -> (Vec<DocumentView>, Vec<DocumentRefusal>) {
        let shared = self.shared.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let mut added = Vec::new();
            let mut refused = Vec::new();
            for path in paths {
                let name = path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                match shared.documents.add_path(&path) {
                    Ok(info) => {
                        info!(document = %info.id, kind = ?info.kind, bytes = info.size, ?via, "document added");
                        added.push(DocumentView::from(&info));
                    }
                    Err(error) => refused.push(DocumentRefusal {
                        name,
                        reason: error.to_string(),
                    }),
                }
            }
            (added, refused)
        })
        .await
        .unwrap_or_else(|_| (Vec::new(), Vec::new()));
        self.shared.emit(CoreEvent::DocumentsAdded {
            via,
            added: outcome.0.clone(),
            refused: outcome.1.clone(),
        });
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_ocr_result_is_an_explicit_read_failure() {
        for pages in [vec![], vec![" \n\t".to_owned()]] {
            let out = vw_mistral::OcrOutput {
                pages,
                model: "test-ocr".to_owned(),
                pages_processed: Some(1),
            };
            assert!(ocr_pages(out).err().unwrap().contains("No readable text"));
        }
        let out = vw_mistral::OcrOutput {
            pages: vec!["Actual document text".to_owned()],
            model: "test-ocr".to_owned(),
            pages_processed: Some(1),
        };
        assert_eq!(ocr_pages(out).unwrap().pages, ["Actual document text"]);
    }
}
