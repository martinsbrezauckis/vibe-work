//! Optional Files read/stat path for session-owned video handles. Every open checks the saved file version;
//! the existing UI image viewer consumes `FileContent` and never needs a native video decoder.
use crate::{CommandResult, Core, CoreError, ErrorCode, files_view::FilesResult};
impl Core {
    pub(crate) async fn video_file(
        &self,
        session: &str,
        file: &str,
        stat: bool,
    ) -> Result<CommandResult, CoreError> {
        if self.shared.store.view(session).is_none() {
            return Err(CoreError::not_loaded(session));
        }
        let tools = self.shared.agent_tools.clone().ok_or_else(|| {
            CoreError::new(ErrorCode::NotFound, "Saved video frames are unavailable.")
        })?;
        let (session, file) = (session.to_owned(), file.to_owned());
        let content =
            tokio::task::spawn_blocking(move || tools.video_history.read(&session, &file))
                .await
                .map_err(|_| {
                    CoreError::new(ErrorCode::Internal, "Saved video frames could not be read.")
                })?;
        let result = if stat {
            FilesResult::Stat {
                exists: content.is_ok(),
                fingerprint: content.ok().map(|c| c.fingerprint),
            }
        } else {
            FilesResult::Content {
                content: content.map_err(|e| CoreError::new(ErrorCode::NotFound, e))?,
            }
        };
        Ok(CommandResult::Files { result })
    }
}
