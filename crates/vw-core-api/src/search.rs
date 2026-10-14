//! The `search` command: messages from the index (`search_index`), files from the sessions'
//! folders (`search_files`).
//!
//! Depends on: `search_index`, `search_files`, `search_text`, `names` (session folders). Called by: `dispatch`.
//! Invariants: the Debug API and the window get the same answers (the Debug API acts as the user); file
//! search runs off the async runtime; only folders sessions work in are searched.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::search_files::{Folder, search as search_files};
use crate::search_text::Query;
use crate::search_view::{SearchRequest, SearchResult};

/// Hits returned when the request names no limit.
const DEFAULT_LIMIT: u32 = 50;
/// Most hits one request may ask for.
const MAX_LIMIT: u32 = 200;

fn limit_of(limit: Option<u32>) -> usize {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT) as usize
}

impl Core {
    /// The folders a file search covers: one session's (its folder and workspace roots) or every known session's,
    /// each once, existing folders only.
    fn search_folders(&self, session_id: Option<&str>) -> Vec<Folder> {
        let mut by_path: BTreeMap<PathBuf, Option<String>> = BTreeMap::new();
        match session_id {
            Some(id) => {
                if let Some(cwd) = self.shared.names.cwd_of(&self.shared.store, id) {
                    by_path.insert(PathBuf::from(cwd), Some(id.to_owned()));
                }
                if let Some(view) = self.shared.store.view(id)
                    && let Some(roots) = view
                        .state
                        .pointer("/session/workspaceRoots")
                        .and_then(serde_json::Value::as_array)
                {
                    for root in roots.iter().filter_map(serde_json::Value::as_str) {
                        by_path
                            .entry(PathBuf::from(root))
                            .or_insert(Some(id.to_owned()));
                    }
                }
            }
            None => {
                for (id, cwd) in self.shared.names.session_folders(&self.shared.store) {
                    // Loaded sessions come first, so an open session is the one a hit opens in.
                    by_path.entry(PathBuf::from(cwd)).or_insert(Some(id));
                }
            }
        }
        by_path
            .into_iter()
            .filter(|(path, _)| path.is_absolute() && path.is_dir())
            .map(|(path, session_id)| Folder { path, session_id })
            .collect()
    }

    /// Executes one `search` request.
    ///
    /// # Errors
    /// `invalidParams` for an empty query; `internal` when the file search task fails.
    pub(crate) async fn search(&self, request: SearchRequest) -> Result<SearchResult, CoreError> {
        let empty = || CoreError::new(ErrorCode::InvalidParams, "the query is empty");
        match request {
            SearchRequest::Messages {
                query,
                session_id,
                limit,
            } => {
                if Query::new(&query).is_none() {
                    return Err(empty());
                }
                let (hits, total, index) = self
                    .search_messages(&query, session_id.as_deref(), limit_of(limit))
                    .await;
                Ok(SearchResult::Messages { hits, total, index })
            }
            SearchRequest::Files {
                query,
                session_id,
                limit,
            } => {
                let prepared = Query::new(&query).ok_or_else(empty)?;
                let folders = self.search_folders(session_id.as_deref());
                let names: Vec<String> = folders
                    .iter()
                    .map(|f| f.path.display().to_string())
                    .collect();
                let limit = limit_of(limit);
                let found =
                    tokio::task::spawn_blocking(move || search_files(&folders, &prepared, limit))
                        .await
                        .map_err(|e| {
                            CoreError::new(ErrorCode::Internal, format!("file search failed: {e}"))
                        })?;
                Ok(SearchResult::Files {
                    hits: found.hits,
                    total: found.total,
                    folders: names,
                    scanned: found.scanned,
                    truncated: found.truncated,
                })
            }
            SearchRequest::Status {} => {
                self.kick_index();
                Ok(SearchResult::Status {
                    index: self.shared.search.status(),
                })
            }
        }
    }
}
