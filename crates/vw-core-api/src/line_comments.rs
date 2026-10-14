//! The `lineComments` command: the user's comments on lines of a session's Changes diff, stored in
//! app data (never in the repository) and placed in the diff by content (`vw_changes::locate`).
//!
//! Depends on: vw-changes (store, anchors), `changes_ops` (`changes_read`), `line_comments_view`. Called by:
//! `dispatch` (`Command::LineComments`), `changes_slots` (session delete).
//! Invariants:
//! - The quoted lines and the anchor come from the core's own fresh diff, never from the request; a hunk id the
//!   diff no longer has is refused as `stale` and nothing is stored.
//! - Every origin runs every op directly (the Debug API acts as the user; its dialog is gone).
//! - Comments never change the diff, the baseline or any file (review integrity); outdated comments stay
//!   listed until the user deletes them.

use std::sync::Arc;

use vw_changes::{ContentKind, FileChange, LineCommentError, LineCommentStore, anchor_for, locate};

use crate::changes_view::{ChangesMode, ChangesResult, ChangesView};
use crate::command::CommandResult;
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::event::CoreEvent;
use crate::line_comments_view::{
    LineCommentState, LineCommentView, LineCommentsRequest, LineCommentsResult,
};

fn core_error(error: &LineCommentError) -> CoreError {
    let code = match error {
        LineCommentError::Invalid(_) => ErrorCode::InvalidParams,
        LineCommentError::NotFound => ErrorCode::NotFound,
        LineCommentError::Io(_) => ErrorCode::Internal,
    };
    CoreError::new(code, error.to_string())
}

/// Where `comment` is in `view`: the diff's file, then the run of lines in it.
fn place(view: &ChangesView, comment: &vw_changes::LineComment) -> LineCommentState {
    if view.mode != ChangesMode::Git {
        return LineCommentState::Outdated {
            reason: "this session has no line-by-line review right now".to_owned(),
        };
    }
    let Some(file) = view.files.iter().find(|f| f.path == comment.path) else {
        return LineCommentState::Outdated {
            reason: "the file has no changes against the session's starting point any more"
                .to_owned(),
        };
    };
    match locate(file, &comment.anchor) {
        Some(found) => LineCommentState::Attached {
            hunk_id: found.hunk_id,
            from: found.from,
            to: found.to,
        },
        None => LineCommentState::Outdated {
            reason: "the lines changed or left the diff".to_owned(),
        },
    }
}

fn shown_file<'a>(view: &'a ChangesView, path: &str) -> Result<&'a FileChange, CoreError> {
    let file = view.files.iter().find(|f| f.path == path).ok_or_else(|| {
        CoreError::new(ErrorCode::Stale, "that file is not in the changes any more")
    })?;
    if file.content != ContentKind::Text {
        return Err(CoreError::new(
            ErrorCode::InvalidParams,
            "only lines of a text file can be commented",
        ));
    }
    Ok(file)
}

impl Core {
    fn line_store(&self) -> Result<Arc<LineCommentStore>, CoreError> {
        self.shared.changes.comments.clone().ok_or_else(|| {
            CoreError::new(
                ErrorCode::Internal,
                "the Changes view is off (no data folder)",
            )
        })
    }

    async fn line_view(&self, session_id: &str) -> Result<ChangesView, CoreError> {
        match self.changes_read(session_id).await? {
            ChangesResult::View { view } => Ok(view),
            _ => Err(CoreError::new(
                ErrorCode::Internal,
                "the changes read returned no view",
            )),
        }
    }

    /// Executes one line-comment request (the same for every origin).
    pub(crate) async fn line_comments(
        &self,
        request: LineCommentsRequest,
    ) -> Result<CommandResult, CoreError> {
        let store = self.line_store()?;
        let session_id = request.session_id().to_owned();
        let changed = |shared: &crate::core::Shared| {
            shared.emit(CoreEvent::LineCommentsChanged {
                session_id: session_id.clone(),
            });
        };
        let result = match request {
            LineCommentsRequest::List { .. } => {
                let view = self.line_view(&session_id).await?;
                let comments = store
                    .list(&session_id)
                    .into_iter()
                    .map(|comment| LineCommentView {
                        place: place(&view, &comment),
                        comment,
                    })
                    .collect();
                LineCommentsResult::List { comments }
            }
            LineCommentsRequest::Add {
                path,
                hunk_id,
                from,
                to,
                text,
                ..
            } => {
                let view = self.line_view(&session_id).await?;
                let file = shown_file(&view, &path)?;
                let hunk = file.hunks.iter().find(|h| h.id == hunk_id).ok_or_else(|| {
                    CoreError::new(ErrorCode::Stale, "that hunk changed, refresh the changes")
                })?;
                let (lines, anchor) = anchor_for(hunk, from as usize, to as usize)
                    .map_err(|e| CoreError::new(ErrorCode::InvalidParams, e.to_string()))?;
                let owned = store.clone();
                let (sid, p) = (session_id.clone(), path.clone());
                let comment =
                    tokio::task::spawn_blocking(move || owned.add(&sid, &p, lines, anchor, &text))
                        .await
                        .map_err(|e| {
                            CoreError::new(ErrorCode::Internal, format!("task failed: {e}"))
                        })?
                        .map_err(|e| core_error(&e))?;
                changed(&self.shared);
                let place = place(&view, &comment);
                LineCommentsResult::Added {
                    comment: Box::new(LineCommentView { comment, place }),
                }
            }
            LineCommentsRequest::Remove { id, .. } => {
                let owned = store.clone();
                let sid = session_id.clone();
                tokio::task::spawn_blocking(move || owned.delete(&sid, &id))
                    .await
                    .map_err(|e| CoreError::new(ErrorCode::Internal, format!("task failed: {e}")))?
                    .map_err(|e| core_error(&e))?;
                changed(&self.shared);
                LineCommentsResult::Done {}
            }
            LineCommentsRequest::MarkSent { ids, .. } => {
                let owned = store.clone();
                let sid = session_id.clone();
                tokio::task::spawn_blocking(move || owned.mark_sent(&sid, &ids))
                    .await
                    .map_err(|e| CoreError::new(ErrorCode::Internal, format!("task failed: {e}")))?
                    .map_err(|e| core_error(&e))?;
                changed(&self.shared);
                LineCommentsResult::Done {}
            }
        };
        Ok(CommandResult::LineComments { result })
    }
}
