//! Read-only uncommitted diffs for the Git bar's selected nested repository.
//! Depends on validated Git discovery and vw-changes; called by the Changes dispatcher.
//! Uses separate app-data scratch indices, preserving the session baseline and every user Git index/ref.

use crate::changes_view::{ChangesMode, ChangesResult, ChangesView, core_error};
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::worktrees::blocking;
use std::time::Instant;

impl Core {
    pub(crate) async fn changes_repo(
        &self,
        session_id: &str,
        repo: &str,
    ) -> Result<ChangesResult, CoreError> {
        let (selected, place) = self.git_selected_place(session_id, Some(repo))?;
        if selected == "." {
            return self.changes_read(session_id).await;
        }
        let dir = self
            .shared
            .changes
            .dir
            .clone()
            .ok_or_else(|| CoreError::new(ErrorCode::Internal, "Changes has no data folder"))?;
        let _one = self.shared.git.lock.lock().await;
        self.git_validate_place(session_id, &place)?;
        let id = session_id.to_owned();
        blocking(move || {
            let started = Instant::now();
            let repo = vw_changes::discover(&place.root)
                .map_err(core_error)?
                .ok_or_else(|| CoreError::new(ErrorCode::NotFound, "Repository moved"))?;
            let scratch = vw_changes::SessionDir::open(&dir, &id)
                .map_err(core_error)?
                .child("repositories")
                .map_err(core_error)?
                .child(&vw_changes::fnv_hex(&[selected.as_bytes()]))
                .map_err(core_error)?;
            let diff = vw_changes::diff_against_head(repo.clone(), &scratch).map_err(core_error)?;
            Ok(ChangesResult::View {
                view: ChangesView {
                    session_id: id,
                    mode: ChangesMode::Uncommitted,
                    folder: Some(place.root.to_string_lossy().into_owned()),
                    repo_root: Some(repo.root.to_string_lossy().into_owned()),
                    scope: Some(repo.scope),
                    reason: None,
                    baseline_lost: false,
                    baseline_at_ms: None,
                    files: diff.files,
                    fingerprint: diff.fingerprint,
                    notes: diff.notes,
                    turns: Vec::new(),
                    offer_history: false,
                    watching: false,
                    took_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                },
            })
        })
        .await?
    }
}
